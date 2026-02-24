use crate::{
    block_ingestor::BlockInfo,
    data_availability::DataAvailabilityCursor,
    service::{Daemon, FinishHandle},
    settlement::{
        fact_registration::FactRegistrar, SettlementBackend, SettlementBackendBuilder,
        SettlementCursor,
    },
    storage::PersistantStorage,
    utils::watch_tx,
};
use anyhow::Result;
use log::{debug, info};
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::{
        codec::Decode,
        types::{BlockId, BlockTag, FunctionCall},
    },
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use starknet::accounts::ExecutionEncoding;
use starknet_types_core::felt::Felt;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

const POLLING_INTERVAL: Duration = Duration::from_secs(1);

/// Settlement backend that submits state-root transitions to the Piltover contract.
///
/// Proof verification is delegated to a [`FactRegistrar`], which handles all prover-specific
/// logic and returns the exact [`starknet::core::types::Call`] to submit to Piltover.  This
/// backend only handles transaction submission, confirmation, and cursor emission.
#[derive(Debug)]
pub struct PiltoverSettlementBackend<FR, DB> {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    piltover_address: Felt,
    fact_registrar: FR,
    da_channel: Receiver<DataAvailabilityCursor<BlockInfo>>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
    db: DB,
}

#[derive(Debug)]
pub struct PiltoverSettlementBackendBuilder<FR, DB> {
    rpc_url: Url,
    piltover_address: Felt,
    account_address: Felt,
    account_private_key: Felt,
    fact_registrar: FR,
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

impl<FR, DB> PiltoverSettlementBackend<FR, DB>
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
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;

        Ok(AppchainState::decode(&raw_result)?)
    }

    async fn run(mut self)
    where
        FR: FactRegistrar,
    {
        loop {
            let new_da = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_da = self.da_channel.recv() => new_da,
            };
            let Some(new_da) = new_da else {
                debug!("Data availability channel closed, shutting down");
                break;
            };

            debug!(block_number = new_da.block_number; "Received new DA cursor");

            let call = match self
                .fact_registrar
                .build_settlement_call(new_da.block_number, new_da.pointer)
                .await
            {
                Ok(Some(call)) => call,
                Ok(None) => {
                    info!(block_number = new_da.block_number; "Fact registrar skipped block");
                    continue;
                }
                Err(e) => {
                    log::error!(
                        "Fact registration failed for block {}: {}",
                        new_da.block_number,
                        e
                    );
                    continue;
                }
            };

            let execution = self.account.execute_v3(vec![call]);

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
                block_number = new_da.block_number;
                "Estimated settlement cost: {} STRK", fees.overall_fee
            );

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
                block_number = new_da.block_number,
                transaction_hash:% = format!("{:#064x}", transaction.transaction_hash);
                "Piltover statement transaction sent"
            );

            // TODO: timeout / error handling
            watch_tx(
                &self.provider,
                transaction.transaction_hash,
                POLLING_INTERVAL,
            )
            .await
            .unwrap();

            info!(
                block_number = new_da.block_number,
                transaction_hash:% = format!("{:#064x}", transaction.transaction_hash);
                "Piltover statement transaction confirmed"
            );

            self.db
                .remove_block(new_da.block_number.try_into().unwrap())
                .await
                .unwrap();

            let new_cursor = SettlementCursor {
                block_number: new_da.block_number,
                transaction_hash: transaction.transaction_hash,
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<FR, DB> PiltoverSettlementBackendBuilder<FR, DB> {
    pub fn new(
        rpc_url: Url,
        piltover_address: Felt,
        account_address: Felt,
        account_private_key: Felt,
        fact_registrar: FR,
        db: DB,
    ) -> Self {
        Self {
            rpc_url,
            piltover_address,
            account_address,
            account_private_key,
            fact_registrar,
            da_channel: None,
            cursor_channel: None,
            db,
        }
    }
}

impl<FR, DB> SettlementBackendBuilder for PiltoverSettlementBackendBuilder<FR, DB>
where
    FR: FactRegistrar + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + 'static,
{
    type Backend = PiltoverSettlementBackend<FR, DB>;

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

        Ok(PiltoverSettlementBackend {
            provider,
            account,
            piltover_address: self.piltover_address,
            fact_registrar: self.fact_registrar,
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

impl<FR, DB> SettlementBackend for PiltoverSettlementBackend<FR, DB>
where
    FR: FactRegistrar + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + 'static,
{
    async fn get_block_number(&self) -> Result<Felt> {
        let appchain_state = self.get_state().await?;
        Ok(appchain_state.block_number)
    }
}

impl<FR, DB> Daemon for PiltoverSettlementBackend<FR, DB>
where
    FR: FactRegistrar + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + 'static,
{
    fn shutdown_handle(&self) -> crate::service::ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
