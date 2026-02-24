//! On-chain STARK integrity verifier fact registration.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use integrity::{split_proof, VerifierConfiguration};
use log::{debug, info};
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{BlockId, BlockTag, Call, TransactionReceipt},
    macros::short_string,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use starknet::accounts::ExecutionEncoding;
use starknet_types_core::felt::Felt;
use swiftness::TransformTo;
use url::Url;

use crate::{
    data_availability::DataAvailabilityPointer,
    settlement::fact_registration::{build_update_state_call, FactRegistrar},
    storage::{BlockStatus, PersistantStorage, Step},
    utils::{calculate_output, felt_to_bigdecimal, retry_with_backoff, split_calls, watch_tx},
};

const POLLING_INTERVAL: Duration = Duration::from_secs(1);

/// Verifies a STARK bridge proof against the on-chain integrity verifier contract and then
/// constructs the `update_state` call for the Piltover settlement contract.
///
/// For each block the registrar:
///   1. Fetches the bridge proof from the DB.
///   2. Splits it into integrity-verifier transactions and submits them on-chain.
///   3. Returns the `update_state` [`Call`] with the extracted program output.
///
/// If the block is already in `verified_proof` status the integrity transactions are skipped and
/// only the `update_state` call is returned (idempotent restart behaviour).
#[derive(Debug)]
pub struct IntegrityFactRegistrar<DB> {
    integrity_address: Felt,
    piltover_address: Felt,
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    db: DB,
}

impl<DB> IntegrityFactRegistrar<DB> {
    /// Constructs the registrar, fetching the chain ID from the RPC endpoint.
    pub async fn new(
        rpc_url: Url,
        integrity_address: Felt,
        piltover_address: Felt,
        account_address: Felt,
        account_private_key: Felt,
        db: DB,
    ) -> Result<Self> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url)));
        let chain_id = provider.chain_id().await?;

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key)),
            account_address,
            chain_id,
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        Ok(Self {
            integrity_address,
            piltover_address,
            provider,
            account,
            db,
        })
    }
}

impl<DB> FactRegistrar for IntegrityFactRegistrar<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    fn build_settlement_call(
        &self,
        block_number: u64,
        da_pointer: Option<DataAvailabilityPointer>,
    ) -> impl std::future::Future<Output = Result<Option<Call>>> + Send + '_ {
        async move {
            let block_number_u32: u32 = block_number.try_into()?;

            let proof_bytes = match self.db.get_proof(block_number_u32, Step::Bridge).await {
                Ok(b) => b,
                Err(e) => {
                    debug!(block_number; "No bridge proof found, skipping: {}", e);
                    return Ok(None);
                }
            };
            let raw_proof = String::from_utf8(proof_bytes)?;

            let layout_bridge_proof = swiftness::parse(raw_proof)?.transform_to();
            let program_output = calculate_output(&layout_bridge_proof);

            let status = self.db.get_status(block_number_u32).await?;
            match status {
                BlockStatus::BridgeProofGenerated => {
                    self.submit_integrity_verification(block_number, block_number_u32, &layout_bridge_proof).await?;
                }
                BlockStatus::VerifiedProof => {
                    info!(block_number; "Bridge proof already verified on integrity, skipping verification");
                }
                _ => {
                    debug!(block_number; "Block in unexpected status {:?}, skipping", status);
                    return Ok(None);
                }
            }

            Ok(Some(build_update_state_call(
                self.piltover_address,
                program_output,
                da_pointer,
            )))
        }
    }
}

impl<DB> IntegrityFactRegistrar<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    async fn submit_integrity_verification(
        &self,
        _block_number: u64,
        block_number_u32: u32,
        proof: &swiftness_stark::types::StarkProof,
    ) -> Result<()> {
        let split = split_proof::<swiftness_air::layout::recursive_with_poseidon::Layout>(
            proof.clone(),
        )?;

        let integrity_job_id = SigningKey::from_random().secret_scalar();
        let calls = split
            .into_calls(
                integrity_job_id,
                VerifierConfiguration {
                    layout: short_string!("recursive_with_poseidon"),
                    hasher: short_string!("keccak_160_lsb"),
                    stone_version: short_string!("stone6"),
                    memory_verification: short_string!("relaxed"),
                },
            )
            .collect_calls(self.integrity_address);
        let chunks = split_calls(calls);

        debug!(
            integrity_job_id:% = format!("{:#064x}", integrity_job_id);
            "{} transactions to integrity verifier generated",
            chunks.len()
        );

        let mut nonce = self.account.get_nonce().await?;
        let mut total_fee = Felt::ZERO;
        let proof_start = Instant::now();

        for (ind, chunk) in chunks.iter().enumerate() {
            let execution = self.account.execute_v3(chunk.to_owned()).nonce(nonce);
            let tx = retry_with_backoff(
                || execution.send(),
                "integrity_verification",
                3,
                Duration::from_secs(3),
            )
            .await?;

            debug!(
                "[{} / {}] Integrity verification transaction sent: {:#064x}",
                ind + 1,
                chunks.len(),
                tx.transaction_hash
            );

            let receipt =
                watch_tx(&self.provider, tx.transaction_hash, POLLING_INTERVAL).await?;

            let fee = match &receipt.receipt {
                TransactionReceipt::Invoke(r) => &r.actual_fee,
                TransactionReceipt::L1Handler(r) => &r.actual_fee,
                TransactionReceipt::Declare(r) => &r.actual_fee,
                TransactionReceipt::Deploy(r) => &r.actual_fee,
                TransactionReceipt::DeployAccount(r) => &r.actual_fee,
            };

            debug!(
                transaction_hash:% = format!("{:#064x}", tx.transaction_hash);
                "[{} / {}] Integrity verification transaction confirmed",
                ind + 1,
                chunks.len()
            );

            nonce += Felt::ONE;
            total_fee += fee.amount;
        }

        let elapsed = Instant::now().duration_since(proof_start).as_secs_f32();
        info!(
            "Proof verified on integrity in {:.2}s. Total cost: {} STRK",
            elapsed,
            felt_to_bigdecimal(total_fee, 18)
        );

        self.db
            .set_status(block_number_u32, "verified_proof".to_string())
            .await?;

        Ok(())
    }
}
