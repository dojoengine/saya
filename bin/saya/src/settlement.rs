//! TEE settlement backend — submits `PiltoverInput::TeeInput` to the Piltover contract.
//!
//! Receives a [`TeeProof`] from the prover stage, decodes the SP1 proof, builds the
//! `TEEInput` calldata, and calls `update_state` on the Piltover contract.
//!
//! Calldata format for `PiltoverInput::TeeInput(TEEInput)` (Cairo enum variant 2):
//!   [2, sp1_proof_len, sp1_proof_felt_0, ..., prev_state_root, state_root,
//!    prev_block_hash, block_hash, prev_block_number, block_number]

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use katana_tee_client::{OnchainProof, StarknetCalldata};
use log::{debug, info};
use saya_core::{
    prover::TeeProof,
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementCursor, TeeSettlementBackendBuilder},
};
use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    core::types::{BlockId, BlockTag, Call, Felt, FunctionCall, TransactionReceipt},
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

const POLLING_INTERVAL: Duration = Duration::from_secs(2);

/// Builds `PiltoverInput::TeeInput(TEEInput)` calldata manually, avoiding a direct dependency on
/// the `piltover` crate which pulls in a conflicting version of `starknet-types-core`.
///
/// Cairo enum serialization: `[variant_index, ...payload]`
/// Variant 2 = `TeeInput`.
/// `TEEInput` serialization: `[sp1_proof_len, ...sp1_proof, prev_state_root, state_root,
///                              prev_block_hash, block_hash, prev_block_number, block_number]`
fn build_tee_calldata(proof: &TeeProof) -> Result<Vec<Felt>> {
    let onchain_proof = OnchainProof::decode_json(&proof.data)?;
    let sp1_felts_raw = StarknetCalldata::from_proof(&onchain_proof)?.to_felts()?;

    // Convert from katana-tee's starknet Felt to bin/saya-tee's starknet Felt via bytes.
    let sp1_proof: Vec<Felt> = sp1_felts_raw
        .iter()
        .map(|f| {
            let bytes = f.to_bytes_be();
            Felt::from_bytes_be(&bytes)
        })
        .collect();

    // Convert saya-core Felt (starknet-types-core v0.2.1) to bin/saya-tee Felt (fork) via bytes.
    let to_felt =
        |f: starknet_types_core::felt::Felt| -> Felt { Felt::from_bytes_be(&f.to_bytes_be()) };

    let mut calldata: Vec<Felt> = Vec::with_capacity(2 + 1 + sp1_proof.len() + 6);
    // Enum variant index 2 = TeeInput
    calldata.push(Felt::from(2u64));
    // Vec<Felt> is serialised as [len, elem0, elem1, ...]
    calldata.push(Felt::from(sp1_proof.len() as u64));
    calldata.extend(sp1_proof);
    calldata.push(to_felt(proof.prev_state_root));
    calldata.push(to_felt(proof.state_root));
    calldata.push(to_felt(proof.prev_block_hash));
    calldata.push(to_felt(proof.block_hash));
    calldata.push(Felt::from(proof.prev_block_number));
    calldata.push(Felt::from(proof.block_number));

    Ok(calldata)
}

/// Settlement backend that submits TEE proofs to the Piltover contract via `update_state`.
#[derive(Debug)]
pub struct TeePiltoverSettlementBackend {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    piltover_address: Felt,
    proof_channel: Receiver<TeeProof>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeePiltoverSettlementBackendBuilder {
    rpc_url: Url,
    // Stored as raw bytes to avoid starknet-types-core version mismatch between
    // the CLI (saya-core v0.2.1) and the dojoengine fork used in settlement.
    piltover_address: [u8; 32],
    account_address: [u8; 32],
    account_private_key: [u8; 32],
    proof_channel: Option<Receiver<TeeProof>>,
    cursor_channel: Option<Sender<SettlementCursor>>,
}

impl TeePiltoverSettlementBackendBuilder {
    /// Accepts `starknet_types_core::felt::Felt` from the CLI and converts via bytes.
    pub fn new(
        rpc_url: Url,
        piltover_address: starknet_types_core::felt::Felt,
        account_address: starknet_types_core::felt::Felt,
        account_private_key: starknet_types_core::felt::Felt,
    ) -> Self {
        Self {
            rpc_url,
            piltover_address: piltover_address.to_bytes_be(),
            account_address: account_address.to_bytes_be(),
            account_private_key: account_private_key.to_bytes_be(),
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

        let piltover_address = Felt::from_bytes_be(&self.piltover_address);
        let account_address = Felt::from_bytes_be(&self.account_address);
        let account_private_key = Felt::from_bytes_be(&self.account_private_key);

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key)),
            account_address,
            chain_id,
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        Ok(TeePiltoverSettlementBackend {
            provider,
            account,
            piltover_address,
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

    async fn run(mut self) {
        loop {
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

            debug!(block_number = proof.block_number; "Received TEE proof for settlement");

            let calldata = match build_tee_calldata(&proof) {
                Ok(c) => c,
                Err(e) => {
                    log::error!(
                        "Failed to build TEE calldata for block {}: {}",
                        proof.block_number,
                        e
                    );
                    continue;
                }
            };

            let call = Call {
                to: self.piltover_address,
                selector: selector!("update_state"),
                calldata,
            };

            let execution = self.account.execute_v3(vec![call]);

            let fees = match execution.estimate_fee().await {
                Ok(f) => f,
                Err(e) => {
                    log::error!(
                        "Fee estimation failed for block {}: {}",
                        proof.block_number,
                        e
                    );
                    continue;
                }
            };
            debug!(block_number = proof.block_number; "Estimated settlement cost: {} STRK", fees.overall_fee);

            let transaction = match execution.send().await {
                Ok(t) => t,
                Err(e) => {
                    log::error!(
                        "Settlement transaction failed for block {}: {}",
                        proof.block_number,
                        e
                    );
                    continue;
                }
            };
            info!(
                block_number = proof.block_number,
                transaction_hash:% = format!("{:#064x}", transaction.transaction_hash);
                "Piltover TEE update_state transaction sent"
            );

            match self.watch_tx(transaction.transaction_hash).await {
                Ok(()) => {}
                Err(e) => {
                    log::error!(
                        "Settlement tx confirmation failed for block {}: {}",
                        proof.block_number,
                        e
                    );
                    continue;
                }
            }

            info!(
                block_number = proof.block_number,
                transaction_hash:% = format!("{:#064x}", transaction.transaction_hash);
                "Piltover TEE update_state transaction confirmed"
            );

            let new_cursor = SettlementCursor {
                block_number: proof.block_number,
                transaction_hash: {
                    // Convert from fork Felt to saya-core Felt via bytes.
                    let bytes = transaction.transaction_hash.to_bytes_be();
                    starknet_types_core::felt::Felt::from_bytes_be(&bytes)
                },
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
    async fn get_block_number(&self) -> Result<starknet_types_core::felt::Felt> {
        let felt = self.get_piltover_block_number().await?;
        let bytes = felt.to_bytes_be();
        Ok(starknet_types_core::felt::Felt::from_bytes_be(&bytes))
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
