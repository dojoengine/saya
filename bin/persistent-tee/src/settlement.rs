//! TEE settlement backend — submits `PiltoverInput::TeeInput` to the Piltover contract.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use cainome::cairo_serde::{CairoSerde, ContractAddress};
use katana_tee_client::{OnchainProof, StarknetCalldata};
use piltover::{MessageToAppchain, MessageToStarknet, PiltoverInput, TEEInput};
use sha3::{Digest, Keccak256};
use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    core::types::{BlockId, BlockTag, Call, Felt, FunctionCall, TransactionReceipt},
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, warn};
use url::Url;

use saya_core::{
    prover::TeeProof,
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementCursor, TeeSettlementBackendBuilder},
    tee::{L1ToL2Message, L2ToL1Message},
};

const POLLING_INTERVAL: Duration = Duration::from_secs(2);

/// Computes the Starknet L1→L2 message hash using keccak256.
///
/// Matches the Ethereum StarknetMessaging.sol formula:
/// `keccak256(abi.encodePacked(from_address, to_address, nonce, selector, payload.length, payload))`
/// where `from_address` is a 20-byte Ethereum address (lower 20 bytes of the felt252).
fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(&msg.from_address.to_bytes_be()[12..]);
    hasher.update(msg.to_address.to_bytes_be());
    hasher.update(msg.nonce.to_bytes_be());
    hasher.update(msg.selector.to_bytes_be());
    hasher.update(Felt::from(msg.payload.len() as u64).to_bytes_be());
    for p in &msg.payload {
        hasher.update(p.to_bytes_be());
    }
    Felt::from_bytes_be(&hasher.finalize().into())
}

fn messages_to_starknet(msgs: &[L2ToL1Message]) -> Vec<MessageToStarknet> {
    msgs.iter()
        .map(|m| MessageToStarknet {
            from_address: ContractAddress(m.from_address),
            to_address: ContractAddress(m.to_address),
            payload: m.payload.clone(),
        })
        .collect()
}

fn messages_to_appchain(msgs: &[L1ToL2Message]) -> Vec<MessageToAppchain> {
    msgs.iter()
        .map(|m| MessageToAppchain {
            from_address: ContractAddress(m.from_address),
            to_address: ContractAddress(m.to_address),
            nonce: m.nonce,
            selector: m.selector,
            payload: m.payload.clone(),
        })
        .collect()
}

/// Builds `PiltoverInput::TeeInput` calldata using the piltover bindgen.
///
/// In real proving mode (`mock_prove = false`), `proof.data` is a JSON-encoded
/// `OnchainProof` produced by the SP1 prover network, and is converted to
/// Garaga calldata via [`StarknetCalldata::from_proof`].
///
/// In mock proving mode (`mock_prove = true`), `proof.data` is a raw
/// big-endian felt buffer produced by [`crate::mock_proof::serialize_mock_journal`]
/// — a Cairo-Serde-serialized stub `VerifierJournal` — which the paired
/// `piltover_mock_amd_tee_registry` contract decodes as-is. We forward the
/// felts directly to `TEEInput.sp1_proof` without going through `OnchainProof`.
fn build_tee_calldata(proof: &TeeProof, mock_prove: bool) -> Result<Vec<Felt>> {
    let sp1_proof: Vec<Felt> = if mock_prove {
        crate::mock_proof::bytes_to_felts(&proof.data)
            .ok_or_else(|| anyhow::anyhow!("mock proof data length is not a multiple of 32"))?
    } else {
        let onchain_proof = OnchainProof::decode_json(&proof.data)?;
        StarknetCalldata::from_proof(&onchain_proof)?
            .to_felts()?
            .iter()
            .map(|f| Felt::from_bytes_be(&f.to_bytes_be()))
            .collect()
    };

    let l1_to_l2_msg_hashes: Vec<Felt> = proof
        .l1_to_l2_messages
        .iter()
        .map(compute_l1_to_l2_msg_hash)
        .collect();

    let tee_input = TEEInput {
        sp1_proof,
        prev_state_root: proof.prev_state_root,
        state_root: proof.state_root,
        prev_block_hash: proof.prev_block_hash,
        block_hash: proof.block_hash,
        prev_block_number: proof.prev_block_number,
        block_number: proof.block_number,
        messages_commitment: proof.messages_commitment,
        messages_to_starknet: messages_to_starknet(&proof.l2_to_l1_messages),
        messages_to_appchain: messages_to_appchain(&proof.l1_to_l2_messages),
        l1_to_l2_msg_hashes,
        katana_tee_config_hash: proof.katana_tee_config_hash,
    };

    Ok(PiltoverInput::cairo_serialize(&PiltoverInput::TeeInput(
        tee_input,
    )))
}

/// What to do with a proof, given the Piltover contract's current on-chain block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettlementAction {
    /// Already settled on-chain (e.g. resumed after a restart, or a re-proved block)
    /// — skip it and just advance the local cursor.
    AlreadySettled,
    /// The chain is exactly at this proof's parent — safe to submit `update_state`.
    Submit,
    /// The chain hasn't reached this proof's parent yet (a prior settlement is still
    /// landing, or `latest` lags `pre_confirmed`) — wait and recheck; never skip.
    WaitForParent,
}

/// The block number a freshly deployed Piltover contract reports before it has settled
/// any block: `-1` in the field, i.e. `PRIME - 1` ([`Felt::MAX`]). The first proof a
/// chain ever submits is block 0, whose `prev_block_number` is also this sentinel.
const FRESH_PILTOVER_BLOCK: Felt = Felt::MAX;

/// Decide whether to submit, wait, or skip a proof.
///
/// Submitting only when the chain is exactly at the proof's parent keeps settlement
/// strictly in order: it never sends an `update_state` the contract would reject with
/// "invalid block number", and never skips a gap. Submitting out of order — or
/// dropping the failed block and moving on — is what wedged the pipeline; see the
/// unit tests below.
fn settlement_action(onchain_block: Felt, proof_prev: Felt, proof_block: Felt) -> SettlementAction {
    // A fresh Piltover (block == the -1 sentinel) has settled nothing yet, so no block
    // can be "already settled" and the only submittable proof is the genesis block
    // (block 0, whose parent is also the sentinel). Handle it explicitly: comparing the
    // sentinel numerically would make it look `>=` every block and skip them all, so a
    // fresh chain would never start settling.
    if onchain_block == FRESH_PILTOVER_BLOCK {
        return if proof_prev == FRESH_PILTOVER_BLOCK {
            SettlementAction::Submit
        } else {
            SettlementAction::WaitForParent
        };
    }

    if onchain_block >= proof_block {
        SettlementAction::AlreadySettled
    } else if onchain_block == proof_prev {
        SettlementAction::Submit
    } else {
        SettlementAction::WaitForParent
    }
}

/// Settlement backend that submits TEE proofs to the Piltover contract via `update_state`.
#[derive(Debug)]
pub struct TeePiltoverSettlementBackend {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    piltover_address: Felt,
    /// When `true`, decode `TeeProof.data` as a raw felt buffer (mock journal)
    /// instead of `OnchainProof` JSON. Must be paired with the upstream
    /// `TeeProver` running with `mock_prove = true`.
    mock_prove: bool,
    proof_channel: Receiver<TeeProof>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeePiltoverSettlementBackendBuilder {
    rpc_url: Url,
    piltover_address: Felt,
    account_address: Felt,
    account_private_key: Felt,
    mock_prove: bool,
    proof_channel: Option<Receiver<TeeProof>>,
    cursor_channel: Option<Sender<SettlementCursor>>,
}

impl TeePiltoverSettlementBackendBuilder {
    pub fn new(
        rpc_url: Url,
        piltover_address: Felt,
        account_address: Felt,
        account_private_key: Felt,
        mock_prove: bool,
    ) -> Self {
        Self {
            rpc_url,
            piltover_address,
            account_address,
            account_private_key,
            mock_prove,
            proof_channel: None,
            cursor_channel: None,
        }
    }
}

impl TeeSettlementBackendBuilder for TeePiltoverSettlementBackendBuilder {
    type Backend = TeePiltoverSettlementBackend;

    async fn build(self) -> Result<Self::Backend> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(self.rpc_url)));
        let chain_id = provider.chain_id().await?;

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(self.account_private_key)),
            self.account_address,
            chain_id,
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        Ok(TeePiltoverSettlementBackend {
            provider,
            account,
            piltover_address: self.piltover_address,
            mock_prove: self.mock_prove,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn proof_channel(mut self, proof_channel: Receiver<TeeProof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<SettlementCursor>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl TeePiltoverSettlementBackend {
    async fn get_piltover_block_number(&self) -> Result<Felt> {
        let raw = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.piltover_address,
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        // AppchainState: [state_root, block_number, block_hash] — block_number is index 1.
        raw.get(1)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("get_state returned fewer than 2 felts"))
    }

    async fn watch_tx(&self, tx_hash: Felt) -> Result<()> {
        loop {
            tokio::time::sleep(POLLING_INTERVAL).await;
            match self.provider.get_transaction_receipt(tx_hash).await {
                Ok(receipt) => match receipt.receipt {
                    TransactionReceipt::Invoke(r) => {
                        use starknet::core::types::ExecutionResult;
                        match r.execution_result {
                            ExecutionResult::Succeeded => return Ok(()),
                            ExecutionResult::Reverted { reason } => {
                                return Err(anyhow::anyhow!("Transaction reverted: {reason}"))
                            }
                        }
                    }
                    _ => return Ok(()),
                },
                Err(starknet::providers::ProviderError::StarknetError(
                    starknet::core::types::StarknetError::TransactionHashNotFound,
                )) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Sleep one polling interval; returns `true` if shutdown was requested while
    /// waiting, so retry loops can stop promptly instead of hanging.
    async fn cooldown(&self) -> bool {
        tokio::select! {
            _ = self.finish_handle.shutdown_requested() => true,
            _ = tokio::time::sleep(POLLING_INTERVAL) => false,
        }
    }

    async fn run(mut self) {
        'outer: loop {
            let proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                p = self.proof_channel.recv() => match p {
                    Some(p) => p,
                    None => {
                        debug!("Proof channel closed, shutting down");
                        break;
                    }
                },
            };

            // Calldata is a pure function of the proof; a build failure means a
            // malformed proof and retrying can't help, so skip it. The settlement
            // errors below are the opposite — transient — so we RETRY the same proof
            // and never drop it. Dropping a proof leaves a permanent gap: every later
            // block's `prev_block_number` then mismatches the on-chain state and the
            // contract rejects it forever ("State: invalid block number").
            let calldata = match build_tee_calldata(&proof, self.mock_prove) {
                Ok(c) => c,
                Err(e) => {
                    error!(
                        "Failed to build TEE calldata for block {}: {}",
                        proof.block_number.to_hex_string(),
                        e
                    );
                    continue;
                }
            };

            // Settle strictly in order. Only submit `update_state` once the Piltover
            // contract's on-chain block equals this proof's parent; otherwise wait —
            // a prior settlement is still landing, or `latest` lags `pre_confirmed`
            // (fee estimation runs against `latest`, so submitting early reverts with
            // "invalid block number"). Retry transient failures rather than advancing
            // to the next proof. A `None` result means the block was already settled
            // on-chain (e.g. resumed after a restart), so we just advance the cursor.
            let tx_hash: Option<Felt> = loop {
                if self.finish_handle.is_shutdown_requested() {
                    break 'outer;
                }

                let onchain = match self.get_piltover_block_number().await {
                    Ok(b) => b,
                    Err(e) => {
                        warn!(
                            "Failed to read Piltover block for {}: {}; retrying",
                            proof.block_number.to_hex_string(),
                            e
                        );
                        if self.cooldown().await {
                            break 'outer;
                        }
                        continue;
                    }
                };

                match settlement_action(onchain, proof.prev_block_number, proof.block_number) {
                    SettlementAction::AlreadySettled => {
                        debug!(
                            "Block {} already settled on-chain; advancing cursor",
                            proof.block_number.to_hex_string()
                        );
                        break None;
                    }
                    SettlementAction::WaitForParent => {
                        // Parent not yet on-chain — wait and recheck; do NOT skip ahead.
                        if self.cooldown().await {
                            break 'outer;
                        }
                        continue;
                    }
                    SettlementAction::Submit => {}
                }

                let call = Call {
                    to: self.piltover_address,
                    selector: selector!("update_state"),
                    calldata: calldata.clone(),
                };
                let execution = self.account.execute_v3(vec![call]);

                if let Err(e) = execution.estimate_fee().await {
                    warn!(
                        "Fee estimation failed for block {}: {}; retrying",
                        proof.block_number.to_hex_string(),
                        e
                    );
                    if self.cooldown().await {
                        break 'outer;
                    }
                    continue;
                }
                let transaction = match execution.send().await {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(
                            "Settlement transaction failed for block {}: {}; retrying",
                            proof.block_number.to_hex_string(),
                            e
                        );
                        if self.cooldown().await {
                            break 'outer;
                        }
                        continue;
                    }
                };
                match self.watch_tx(transaction.transaction_hash).await {
                    Ok(()) => break Some(transaction.transaction_hash),
                    Err(e) => {
                        warn!(
                            "Settlement tx confirmation failed for block {}: {}; retrying",
                            proof.block_number.to_hex_string(),
                            e
                        );
                        if self.cooldown().await {
                            break 'outer;
                        }
                        continue;
                    }
                }
            };

            let new_cursor = SettlementCursor {
                block_number: u64::try_from(proof.block_number).unwrap_or_else(|_| {
                    panic!(
                        "Block number {} does not fit in u64",
                        proof.block_number.to_hex_string()
                    )
                }),
                transaction_hash: tx_hash.unwrap_or(Felt::ZERO),
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("TeePiltoverSettlementBackend graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl SettlementBackend for TeePiltoverSettlementBackend {
    async fn get_block_number(&self) -> Result<Felt> {
        self.get_piltover_block_number().await
    }
}

impl Daemon for TeePiltoverSettlementBackend {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

#[cfg(test)]
mod tests {
    use super::{settlement_action, SettlementAction};
    use starknet::core::types::Felt;

    fn f(n: u64) -> Felt {
        Felt::from(n)
    }

    /// The Piltover "no block settled yet" sentinel: block_number == -1 (PRIME - 1).
    fn genesis() -> Felt {
        Felt::ZERO - Felt::ONE
    }

    #[test]
    fn submits_when_chain_is_at_the_proofs_parent() {
        // Chain settled up to block 5; the next proof (block 6, parent 5) is the only
        // one that may be submitted.
        assert_eq!(
            settlement_action(f(5), f(5), f(6)),
            SettlementAction::Submit
        );
    }

    #[test]
    fn settles_the_genesis_block_from_the_minus_one_sentinel() {
        // Regression: a fresh Piltover reports its block as -1 (felt PRIME-1). The first
        // proof is block 0 with parent -1. Comparing felts numerically made the huge
        // sentinel look "already settled" >= every block, so settlement never started
        // (it skipped every block). Genesis must be ordered below block 0 and submitted.
        assert_eq!(
            settlement_action(genesis(), genesis(), f(0)),
            SettlementAction::Submit
        );
        // From genesis, block 1 (parent 0) must still wait for block 0 first.
        assert_eq!(
            settlement_action(genesis(), f(0), f(1)),
            SettlementAction::WaitForParent
        );
    }

    #[test]
    fn waits_for_parent_instead_of_settling_out_of_order() {
        // Regression for the settlement wedge: the prover/ingestor race ahead while the
        // chain is stuck at block 5, offering block 10 (parent 9). Submitting it would
        // revert with "State: invalid block number"; the old code then dropped the
        // proof and every later block's parent mismatched, cascading into a permanent
        // stall. The backend must WAIT for the parent — never submit out of order.
        assert_eq!(
            settlement_action(f(5), f(9), f(10)),
            SettlementAction::WaitForParent
        );
        // Even one block ahead must wait: `latest` lags `pre_confirmed`, so the parent
        // may not be visible to fee estimation yet.
        assert_eq!(
            settlement_action(f(5), f(6), f(7)),
            SettlementAction::WaitForParent
        );
    }

    #[test]
    fn skips_blocks_already_settled_on_chain() {
        // Resumed after a restart with a stale local cursor: the chain is at 8, but a
        // re-proved block 6 arrives — skip it (idempotent), don't re-settle.
        assert_eq!(
            settlement_action(f(8), f(5), f(6)),
            SettlementAction::AlreadySettled
        );
        // The proof's own block already being on-chain counts as settled too.
        assert_eq!(
            settlement_action(f(6), f(5), f(6)),
            SettlementAction::AlreadySettled
        );
    }
}
