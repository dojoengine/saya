use std::{sync::Arc, time::Duration};

use anyhow::Result;
use log::{debug, error, info, trace};
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use tokio::{
    sync::{
        mpsc::{self, Sender},
        Mutex,
    },
    task,
    time::sleep,
};
use url::Url;

use crate::{
    block_ingestor::{BlockIngestor, BlockIngestorBuilder, NewBlock},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

use super::BlockPieGenerator;

const PROVE_BLOCK_FAILURE_BACKOFF: Duration = Duration::from_secs(5);
const BLOCK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TASK_BUFFER_SIZE: usize = 10;
const WORKER_COUNT: usize = 5;
const MAX_RETRIES: usize = 3;

/// A block ingestor which collects new blocks by polling a Starknet RPC endpoint.
#[derive(Debug)]
pub struct PollingBlockIngestor<S, B> {
    rpc_url: Url,
    snos: S,
    current_block: u64,
    channel: Sender<NewBlock>,
    finish_handle: FinishHandle,
    block_pie_generator: B,
}

#[derive(Debug)]
pub struct PollingBlockIngestorBuilder<S, B> {
    rpc_url: Url,
    snos: S,
    start_block: Option<u64>,
    channel: Option<Sender<NewBlock>>,
    block_pie_generator: B,
}

impl<S, B> PollingBlockIngestor<S, B>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    B: BlockPieGenerator + Send + Sync + Clone + 'static,
{
    /// Fetches the latest block number from the StarkNet RPC.
    async fn get_latest_block(&self) -> Option<u64> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone()));
        match provider.block_number().await {
            Ok(block_number) => Some(block_number),
            Err(err) => {
                error!("Failed to fetch latest block: {}", err);
                None
            }
        }
    }

    /// Worker function: proves a block and sends the result.
    async fn worker(
        task_rx: Arc<Mutex<mpsc::Receiver<u64>>>,
        block_pie_generator: B,
        finish_handle: FinishHandle,
        rpc_url: Url,
        channel: mpsc::Sender<NewBlock>,
        snos: S,
    ) where
        S: AsRef<[u8]> + Send + Sync + 'static,
        B: BlockPieGenerator + Send + Sync + 'static,
    {   
        
        loop {
            let block_number = if let Some(block_number) = task_rx.lock().await.recv().await {
                block_number
            } else {
                break;
            };

            if finish_handle.is_shutdown_requested() {
                break;
            }

            let mut retries = 0;
            let pie = loop {
                match block_pie_generator
                    .prove_block(
                        snos.as_ref(),
                        block_number,
                        rpc_url.as_str().trim_end_matches("/rpc/v0_7"),
                    )
                    .await
                {
                    Ok(pie) => break pie,
                    Err(err) => {
                        error!(
                            "Failed to prove block #{} (attempt {}/{}): {}",
                            block_number,
                            retries + 1,
                            MAX_RETRIES,
                            err
                        );

                        if retries >= MAX_RETRIES {
                            error!("Exceeded max retries for block #{}", block_number);
                            return;
                        }

                        retries += 1;
                        sleep(PROVE_BLOCK_FAILURE_BACKOFF).await;
                    }
                }
            };
            if finish_handle.is_shutdown_requested() {
                break;
            }

            let new_block = NewBlock {
                number: block_number,
                pie,
            };
            info!("Pie generated for block #{}", block_number);
            if channel.send(new_block).await.is_err() {
                error!("Failed to send block #{}", block_number);
            }
        }
    }

    async fn run(mut self) {
        let (task_tx, task_rx) = mpsc::channel(TASK_BUFFER_SIZE);
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(task_rx));

        for _ in 0..WORKER_COUNT {
            let worker_task_rx = task_rx.clone();
            let block_pie_generator = self.block_pie_generator.clone();
            let finish_handle = self.finish_handle.clone();
            let rpc_url = self.rpc_url.clone();
            let channel = self.channel.clone();
            let snos = self.snos.clone();

            workers.push(task::spawn(Self::worker(
                worker_task_rx,
                block_pie_generator,
                finish_handle,
                rpc_url,
                channel,
                snos,
            )));
        }

        // Block Fetching Loop: Waits for valid blocks and sends them to the worker queue
        while !self.finish_handle.is_shutdown_requested() {
            match self.get_latest_block().await {
                Some(latest_block) if latest_block >= self.current_block => {
                    if task_tx.send(self.current_block).await.is_err() {
                        break;
                    }
                    self.current_block += 1;
                }
                _ => {
                    trace!("Block #{} is not available yet", self.current_block);
                    sleep(BLOCK_CHECK_INTERVAL).await;
                }
            }
        }

        drop(task_tx);
        futures_util::future::join_all(workers).await; // Wait for all workers
        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<S, B> PollingBlockIngestorBuilder<S, B> {
    pub fn new(rpc_url: Url, snos: S, block_pie_generator: B) -> Self {
        Self {
            rpc_url,
            snos,
            start_block: None,
            channel: None,
            block_pie_generator,
        }
    }
}

impl<S, B> BlockIngestorBuilder for PollingBlockIngestorBuilder<S, B>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    B: BlockPieGenerator + Send + Sync + Clone + 'static,
{
    type Ingestor = PollingBlockIngestor<S, B>;

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
            block_pie_generator: self.block_pie_generator,
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

impl<S, B> BlockIngestor for PollingBlockIngestor<S, B>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    B: BlockPieGenerator + Send + Sync + Clone + 'static,
{
}

impl<S, B> Daemon for PollingBlockIngestor<S, B>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    B: BlockPieGenerator + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
