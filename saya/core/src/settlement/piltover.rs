use std::{sync::Arc, time::Duration};

use anyhow::Result;
use log::{debug, info};
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::{
        codec::{Decode, Encode},
        types::{BlockId, BlockTag, Call, FunctionCall, U256},
    },
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    data_availability::DataAvailabilityCursor,
    prover::RecursiveProof,
    service::{Daemon, FinishHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
    utils::{calculate_output, felt_to_bigdecimal, watch_tx},
};

#[derive(Debug)]
pub struct PiltoverSettlementBackend {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    contract_address: Felt,
    da_channel: Receiver<DataAvailabilityCursor<RecursiveProof>>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PiltoverSettlementBackendBuilder {
    rpc_url: Url,
    contract_address: Felt,
    account_address: Felt,
    account_private_key: Felt,
    da_channel: Option<Receiver<DataAvailabilityCursor<RecursiveProof>>>,
    cursor_channel: Option<Sender<SettlementCursor>>,
}

#[derive(Debug, Decode)]
struct AppchainState {
    #[allow(unused)]
    state_root: Felt,
    block_number: u64,
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

impl PiltoverSettlementBackend {
    async fn get_state(&self) -> Result<AppchainState> {
        let raw_result = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.contract_address,
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await?;

        Ok(AppchainState::decode(&raw_result)?)
    }

    async fn run(mut self) {
        loop {
            let new_da = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_da = self.da_channel.recv() => new_da,
            };

            // This should be fine for now as DA backends wouldn't drop senders. This might change
            // in the future.
            let new_da = new_da.unwrap();
            debug!("Received new DA cursor");

            let update_state_call = Call {
                to: self.contract_address,
                selector: selector!("update_state"),
                calldata: {
                    let calldata = UpdateStateCalldata {
                        snos_output: calculate_output(&new_da.full_payload.snos_proof),
                        program_output: calculate_output(&new_da.full_payload.layout_bridge_proof),
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
            let fees = execution.estimate_fee().await.unwrap();
            debug!(
                "Estimated settlement transaction cost for block #{}: {} STRK",
                new_da.block_number,
                felt_to_bigdecimal(fees.overall_fee, 18)
            );

            // TODO: wait for transaction to confirm
            // TODO: error handling
            let transaction = execution.send().await.unwrap();
            info!(
                "Piltover statement transaction sent for block #{}: {}",
                new_da.block_number, transaction.transaction_hash
            );

            // TODO: timeout
            // TODO: error handling
            watch_tx(
                &self.provider,
                transaction.transaction_hash,
                Duration::from_secs(2),
            )
            .await
            .unwrap();
            info!(
                "Piltover statement transaction block #{} confirmed: {}",
                new_da.block_number, transaction.transaction_hash
            );

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

impl PiltoverSettlementBackendBuilder {
    pub fn new(
        rpc_url: Url,
        contract_address: Felt,
        account_address: Felt,
        account_private_key: Felt,
    ) -> Self {
        Self {
            rpc_url,
            contract_address,
            account_address,
            account_private_key,
            da_channel: None,
            cursor_channel: None,
        }
    }
}

impl SettlementBackendBuilder for PiltoverSettlementBackendBuilder {
    type Backend = PiltoverSettlementBackend;

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
            contract_address: self.contract_address,
            da_channel: self
                .da_channel
                .ok_or_else(|| anyhow::anyhow!("`da_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn da_channel(mut self, da_channel: Receiver<DataAvailabilityCursor<RecursiveProof>>) -> Self {
        self.da_channel = Some(da_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<SettlementCursor>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl SettlementBackend for PiltoverSettlementBackend {
    async fn get_block_number(&self) -> Result<u64> {
        let appchain_state = self.get_state().await?;
        Ok(appchain_state.block_number)
    }
}

impl Daemon for PiltoverSettlementBackend {
    fn shutdown_handle(&self) -> crate::service::ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
