use std::time::Duration;

use anyhow::Result;
use cairo_vm::types::layout_name::LayoutName;
use log::{debug, error};
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use tokio::sync::mpsc::Sender;
use url::Url;

use crate::{
    block_ingestor::{BlockIngestor, BlockIngestorBuilder, NewBlock},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

const PROVE_BLOCK_FAILURE_BACKOFF: Duration = Duration::from_secs(5);

/// A block ingestor which collects new blocks by polling a Starknet RPC endpoint.
#[derive(Debug)]
pub struct PollingBlockIngestor<S> {
    rpc_url: Url,
    snos: S,
    current_block: u64,
    channel: Sender<NewBlock>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PollingBlockIngestorBuilder<S> {
    rpc_url: Url,
    snos: S,
    start_block: Option<u64>,
    channel: Option<Sender<NewBlock>>,
}

impl<S> PollingBlockIngestor<S>
where
    S: AsRef<[u8]>,
{
    async fn run(mut self) {
        let provider = JsonRpcClient::new(HttpTransport::new(
            Url::parse(self.rpc_url.as_str()).unwrap(),
        ));

        loop {
            let latest_block = provider.block_number().await.unwrap();

            if self.current_block > latest_block {
                debug!(
                    "Current block {} is greater than latest block {}",
                    self.current_block, latest_block
                );

                tokio::select! {
                    _ = self.finish_handle.shutdown_requested() => break,
                    _ = tokio::time::sleep(PROVE_BLOCK_FAILURE_BACKOFF) => continue,
                }
            }

            let pie = match prove_block::prove_block(
                self.snos.as_ref(),
                self.current_block,
                // This is because `snos` expects a base URL to be able to derive `pathfinder` RPC path.
                self.rpc_url.as_str().trim_end_matches("/rpc/v0_7"),
                LayoutName::all_cairo,
                true,
            )
            .await
            // Need to do this as `ProveBlockError::ReExecutionError` is not `Send`
            .map_err(|err| format!("{}", err))
            {
                Ok((pie, _)) => pie,
                Err(err) => {
                    error!("Failed to prove block #{}: {}", self.current_block, err);

                    tokio::select! {
                        _ = self.finish_handle.shutdown_requested() => break,
                        _ = tokio::time::sleep(PROVE_BLOCK_FAILURE_BACKOFF) => continue,
                    }
                }
            };

            debug!("PIE generated for block #{}", self.current_block);

            // No way to hook into `prove_block` for cancellation. The next best thing we can do is
            // to check cancellation immediately after PIE generation.
            if self.finish_handle.is_shutdown_requested() {
                break;
            }

            let new_block = NewBlock {
                number: self.current_block,
                pie,
            };

            // Since the channel is bounded, it's possible
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.channel.send(new_block) => {},
            }

            self.current_block += 1;
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<S> PollingBlockIngestorBuilder<S> {
    pub fn new(rpc_url: Url, snos: S) -> Self {
        Self {
            rpc_url,
            snos,
            start_block: None,
            channel: None,
        }
    }
}

impl<S> BlockIngestorBuilder for PollingBlockIngestorBuilder<S>
where
    S: AsRef<[u8]> + Send + 'static,
{
    type Ingestor = PollingBlockIngestor<S>;

    fn build(self) -> Result<Self::Ingestor> {
        Ok(PollingBlockIngestor {
            rpc_url: self.rpc_url,
            snos: self.snos,
            current_block: self
                .start_block
                .ok_or_else(|| anyhow::anyhow!("`start_block` not set"))?,
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("`channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn start_block(mut self, start_block: u64) -> Self {
        self.start_block = Some(start_block);
        self
    }

    fn channel(mut self, channel: Sender<NewBlock>) -> Self {
        self.channel = Some(channel);
        self
    }
}

impl<S> BlockIngestor for PollingBlockIngestor<S> where S: AsRef<[u8]> + Send + 'static {}

impl<S> Daemon for PollingBlockIngestor<S>
where
    S: AsRef<[u8]> + Send + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
