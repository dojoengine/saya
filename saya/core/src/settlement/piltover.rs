use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use integrity::{split_proof, VerifierConfiguration};
use log::{debug, info};
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::{
        codec::{Decode, Encode},
        types::{BlockId, BlockTag, Call, FunctionCall, TransactionReceipt, U256},
    },
    macros::{selector, short_string},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use starknet_types_core::felt::Felt;
use swiftness::TransformTo;
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    block_ingestor::BlockInfo,
    data_availability::DataAvailabilityCursor,
    service::{Daemon, FinishHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
    storage::PersistantStorage,
    utils::{calculate_output, felt_to_bigdecimal, split_calls, watch_tx},
};

const POLLING_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct PiltoverSettlementBackend<DB> {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    fact_registration: FactRegistrationConfig,
    piltover_address: Felt,
    da_channel: Receiver<DataAvailabilityCursor<BlockInfo>>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
    db: DB,
}

#[derive(Debug)]
pub struct PiltoverSettlementBackendBuilder<DB> {
    rpc_url: Url,
    integrity_address: Option<Felt>,
    skip_fact_registration: bool,
    piltover_address: Felt,
    account_address: Felt,
    account_private_key: Felt,
    da_channel: Option<Receiver<DataAvailabilityCursor<BlockInfo>>>,
    cursor_channel: Option<Sender<SettlementCursor>>,
    db: DB,
}

#[derive(Debug, Decode)]
struct AppchainState {
    #[allow(unused)]
    state_root: Felt,
    block_number: Felt,
    #[allow(unused)]
    block_hash: Felt,
}

#[derive(Debug, Encode)]
struct UpdateStateCalldata {
    snos_output: Vec<Felt>,
    program_output: Vec<Felt>,
    onchain_data_hash: Felt,
    onchain_data_size: U256,
}

#[derive(Debug)]
enum FactRegistrationConfig {
    Integrity(Felt),
    Skipped,
}

impl<DB> PiltoverSettlementBackend<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    async fn get_state(&self) -> Result<AppchainState> {
        let raw_result = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.piltover_address,
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await?;

        Ok(AppchainState::decode(&raw_result)?)
    }

    async fn run(mut self) {
        let mut pending_blocks: BTreeMap<u64, DataAvailabilityCursor<BlockInfo>> = BTreeMap::new();
        loop {
            let last_settled_block = self.get_block_number().await.unwrap();

            let next_to_settle = if last_settled_block == Felt::MAX {
                0
            } else {
                <Felt as TryInto<u64>>::try_into(last_settled_block).unwrap() + 1
            };

            let da = pending_blocks.remove(&next_to_settle);

            let Some(new_da) = da else {
                let new_da = tokio::select! {
                    _ = self.finish_handle.shutdown_requested() => break,
                    new_da = self.da_channel.recv() => new_da,
                };
                let new_da = match new_da {
                    Some(new_da) => new_da,
                    None => {
                        debug!("Data availability channel closed, shutting down");
                        break;
                    }
                };

                pending_blocks.insert(new_da.block_number, new_da.clone());
                continue;
            };

            debug!("Received new DA cursor");
            let layout_bridge_proof = self
                .db
                .get_proof(
                    new_da.block_number.try_into().unwrap(),
                    crate::storage::Step::Bridge,
                )
                .await
                .unwrap();
            let raw_proof = String::from_utf8(layout_bridge_proof).unwrap();
            let layout_bridge_proof = swiftness::parse(raw_proof).unwrap().transform_to();

            match self
                .db
                .get_status(new_da.block_number.try_into().unwrap())
                .await
                .unwrap()
            {
                crate::storage::BlockStatus::BridgeProofGenerated => {
                    match self.fact_registration {
                        FactRegistrationConfig::Integrity(integrity_address) => {
                            // TODO: error handling
                            let split_proof = split_proof::<
                                swiftness_air::layout::recursive_with_poseidon::Layout,
                            >(
                                layout_bridge_proof.clone()
                            )
                            .unwrap();
                            let integrity_job_id = SigningKey::from_random().secret_scalar();
                            let integrity_calls = split_proof
                                .into_calls(
                                    integrity_job_id,
                                    VerifierConfiguration {
                                        layout: short_string!("recursive_with_poseidon"),
                                        hasher: short_string!("keccak_160_lsb"),
                                        stone_version: short_string!("stone6"),
                                        memory_verification: short_string!("relaxed"),
                                    },
                                )
                                .collect_calls(integrity_address);
                            let integrity_call_chunks = split_calls(integrity_calls);
                            debug!(
                                "{} transactions to integrity verifier generated (job id: {:#064x})",
                                integrity_call_chunks.len(),
                                integrity_job_id
                            );

                            // TODO: error handling
                            let mut nonce = self.account.get_nonce().await.unwrap();
                            let mut total_fee = Felt::ZERO;

                            let proof_start = Instant::now();

                            for (ind, chunk) in integrity_call_chunks.iter().enumerate() {
                                let execution =
                                    self.account.execute_v3(chunk.to_owned()).nonce(nonce);
                                let tx = crate::utils::retry_with_backoff(
                                    || execution.send(),
                                    "integrity_verification",
                                    3,
                                    Duration::from_secs(3),
                                )
                                .await
                                .unwrap();
                                debug!(
                                    "[{} / {}] Integrity verification transaction sent: {:#064x}",
                                    ind + 1,
                                    integrity_call_chunks.len(),
                                    tx.transaction_hash
                                );

                                // TODO: error handling
                                let receipt =
                                    watch_tx(&self.provider, tx.transaction_hash, POLLING_INTERVAL)
                                        .await
                                        .unwrap();

                                let fee = match &receipt.receipt {
                                    TransactionReceipt::Invoke(receipt) => &receipt.actual_fee,
                                    TransactionReceipt::L1Handler(receipt) => &receipt.actual_fee,
                                    TransactionReceipt::Declare(receipt) => &receipt.actual_fee,
                                    TransactionReceipt::Deploy(receipt) => &receipt.actual_fee,
                                    TransactionReceipt::DeployAccount(receipt) => {
                                        &receipt.actual_fee
                                    }
                                };

                                debug!(
                                    "[{} / {}] Integrity verification transaction confirmed: {:#064x}",
                                    ind + 1,
                                    integrity_call_chunks.len(),
                                    tx.transaction_hash
                                );

                                nonce += Felt::ONE;
                                total_fee += fee.amount;
                            }

                            let proof_end = Instant::now();
                            info!(
                                "Proof successfully verified on integrity in {:.2} \
                                seconds. Total cost: {} STRK",
                                proof_end.duration_since(proof_start).as_secs_f32(),
                                felt_to_bigdecimal(total_fee, 18)
                            );
                            self.db
                                .set_status(
                                    next_to_settle.try_into().unwrap(),
                                    "verified_proof".to_string(),
                                )
                                .await
                                .unwrap();
                        }
                        FactRegistrationConfig::Skipped => {
                            info!(
                                "On-chain fact-registration skipped for block #{}",
                                new_da.block_number
                            );
                        }
                    }
                }
                crate::storage::BlockStatus::VerifiedProof => {
                    info!(
                        "Block #{} already verified, skipping verification",
                        new_da.block_number
                    );
                }
                _ => {
                    info!(
                        "Block #{} in unexpected state, skipping settlement",
                        new_da.block_number
                    );
                    continue;
                }
            }

            let new_snos_proof = self
                .db
                .get_proof(
                    new_da.block_number.try_into().unwrap(),
                    crate::storage::Step::Snos,
                )
                .await
                .unwrap();

            let new_snos_proof = String::from_utf8(new_snos_proof).unwrap();
            let parsed_snos_proof = swiftness::parse(&new_snos_proof).unwrap().transform_to();
            let snos_output = calculate_output(&parsed_snos_proof);

            let update_state_call = Call {
                to: self.piltover_address,
                selector: selector!("update_state"),
                calldata: {
                    let calldata = UpdateStateCalldata {
                        snos_output,
                        program_output: calculate_output(&layout_bridge_proof),
                        onchain_data_hash: Felt::ZERO,
                        onchain_data_size: U256::from_words(0, 0),
                    };
                    let mut raw_calldata = vec![];

                    // Encoding `UpdateStateCalldata` never fails
                    calldata.encode(&mut raw_calldata).unwrap();

                    raw_calldata
                },
            };

            let execution = self.account.execute_v3(vec![update_state_call]);

            // TODO: error handling
            let fees = crate::utils::retry_with_backoff(
                || execution.estimate_fee(),
                "estimate_fee",
                3,
                Duration::from_secs(3),
            )
            .await
            .unwrap();
            debug!(
                "Estimated settlement transaction cost for block #{}: {} STRK",
                new_da.block_number,
                felt_to_bigdecimal(fees.overall_fee, 18)
            );

            // TODO: wait for transaction to confirm
            // TODO: error handling
            let transaction = crate::utils::retry_with_backoff(
                || execution.send(),
                "settlement",
                3,
                Duration::from_secs(3),
            )
            .await
            .unwrap();
            info!(
                "Piltover statement transaction sent for block #{}: {:#064x}",
                new_da.block_number, transaction.transaction_hash
            );

            // TODO: timeout
            // TODO: error handling
            watch_tx(
                &self.provider,
                transaction.transaction_hash,
                POLLING_INTERVAL,
            )
            .await
            .unwrap();

            info!(
                "Piltover statement transaction block #{} confirmed: {:#064x}",
                new_da.block_number, transaction.transaction_hash
            );

            self.db
                .remove_block(new_da.block_number.try_into().unwrap())
                .await
                .unwrap();
            let new_cursor = SettlementCursor {
                block_number: new_da.block_number,
                transaction_hash: transaction.transaction_hash,
            };

            // Since the channel is bounded, it's possible
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<DB> PiltoverSettlementBackendBuilder<DB> {
    pub fn new(
        rpc_url: Url,
        piltover_address: Felt,
        account_address: Felt,
        account_private_key: Felt,
        db: DB,
    ) -> Self {
        Self {
            rpc_url,
            integrity_address: None,
            skip_fact_registration: false,
            piltover_address,
            account_address,
            account_private_key,
            da_channel: None,
            cursor_channel: None,
            db,
        }
    }

    pub fn integrity_address(mut self, integrity_address: Felt) -> Self {
        self.integrity_address = Some(integrity_address);
        self
    }

    pub fn skip_fact_registration(mut self, skip_fact_registration: bool) -> Self {
        self.skip_fact_registration = skip_fact_registration;
        self
    }
}

impl<DB> SettlementBackendBuilder for PiltoverSettlementBackendBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    type Backend = PiltoverSettlementBackend<DB>;

    async fn build(self) -> Result<Self::Backend> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(self.rpc_url)));
        let chain_id = provider.chain_id().await?;

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(self.account_private_key)),
            self.account_address,
            chain_id,
            starknet::accounts::ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Pending));

        Ok(PiltoverSettlementBackend {
            provider,
            account,
            fact_registration: if self.skip_fact_registration {
                FactRegistrationConfig::Skipped
            } else {
                FactRegistrationConfig::Integrity(
                    self.integrity_address
                        .ok_or_else(|| anyhow::anyhow!("`integrity_address` not set"))?,
                )
            },
            piltover_address: self.piltover_address,
            da_channel: self
                .da_channel
                .ok_or_else(|| anyhow::anyhow!("`da_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            db: self.db,
        })
    }

    fn da_channel(mut self, da_channel: Receiver<DataAvailabilityCursor<BlockInfo>>) -> Self {
        self.da_channel = Some(da_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<SettlementCursor>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl<DB> SettlementBackend for PiltoverSettlementBackend<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    async fn get_block_number(&self) -> Result<Felt> {
        let appchain_state = self.get_state().await?;
        Ok(appchain_state.block_number)
    }
}

impl<DB> Daemon for PiltoverSettlementBackend<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    fn shutdown_handle(&self) -> crate::service::ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
