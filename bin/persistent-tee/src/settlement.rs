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

                if onchain >= proof.block_number {
                    debug!(
                        "Block {} already settled on-chain; advancing cursor",
                        proof.block_number.to_hex_string()
                    );
                    break None;
                }
                if onchain != proof.prev_block_number {
                    // Parent not yet on-chain — wait and recheck; do NOT skip ahead.
                    if self.cooldown().await {
                        break 'outer;
                    }
                    continue;
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
