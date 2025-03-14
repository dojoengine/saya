use std::{sync::Arc, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
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
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    prover::compress_pie,
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{BlockStatus, PersistantStorage, Step},
};
use prove_block::prove_block;

const BLOCK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TASK_BUFFER_SIZE: usize = 4;
const MAX_RETRIES: usize = 3;

/// A block ingestor which collects new blocks by polling a Starknet RPC endpoint.
#[derive(Debug)]
pub struct PollingBlockIngestor<S, DB> {
    rpc_url: Url,
    snos: S,
    current_block: u64,
    channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
}

#[derive(Debug)]
pub struct PollingBlockIngestorBuilder<S, DB> {
    rpc_url: Url,
    snos: S,
    start_block: Option<u64>,
    channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
}

impl<S, DB> PollingBlockIngestor<S, DB>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    /// Fetches the latest block number from the StarkNet RPC.
    async fn get_latest_block(&self) -> Option<u64> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone()));

        let block_number = crate::utils::retry_with_backoff(
            || provider.block_number(),
            "get_latest_block",
            MAX_RETRIES as u32,
            Duration::from_secs(5),
        )
        .await;

        match block_number {
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
        finish_handle: FinishHandle,
        rpc_url: Url,
        channel: mpsc::Sender<BlockInfo>,
        snos: S,
        db: DB,
    ) where
        S: AsRef<[u8]> + Send + Sync + 'static,
        DB: PersistantStorage + Send + Sync + 'static,
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

            db.initialize_block(block_number.try_into().unwrap())
                .await
                .unwrap();

            match db
                .get_pie(block_number.try_into().unwrap(), Step::Snos)
                .await
            {
                Ok(pie_bytes) => match CairoPie::from_bytes(&pie_bytes) {
                    Ok(_pie) => {
                        let new_block = BlockInfo {
                            number: block_number,
                            status: BlockStatus::SnosPieGenerated,
                        };
                        log::trace!("Pie generated for block #{}", block_number);

                        if channel.send(new_block).await.is_err() {
                            error!("Failed to send block #{}", block_number);
                        }
                        continue;
                    }
                    Err(err) => {
                        error!("Failed to parse pie for block #{}: {:?}", block_number, err)
                    }
                },
                Err(err) => {
                    // Not found in db, we continue;
                    trace!("Failed to get pie for block #{}: {:?}", block_number, err);
                }
            }

            let (pie, _) = prove_block(
                snos.as_ref(),
                block_number,
                rpc_url.as_str().trim_end_matches("/rpc/v0_7"),
                cairo_vm::types::layout_name::LayoutName::all_cairo,
                true,
            )
            .await
            .unwrap();

            if finish_handle.is_shutdown_requested() {
                break;
            }

            let new_block = BlockInfo {
                number: block_number,
                status: BlockStatus::SnosPieGenerated,
            };

            let pie_bytes = compress_pie(pie.clone()).await.unwrap();
            let block_number = block_number.try_into().unwrap();

            db.add_pie(block_number, pie_bytes.clone(), Step::Snos)
                .await
                .unwrap();

            info!("Pie generated for block #{}", block_number);

            if channel.send(new_block).await.is_err() {
                error!("Failed to send block #{}", block_number);
            }
        }
    }

    /// Continuously fetches the latest available block and sends it to the worker queue.
    ///
    /// This loop ensures that blocks are processed sequentially while also handling previously
    /// failed blocks. It will continue running until a shutdown request is received.
    ///
    /// # Process:
    /// - Checks if the latest available block is greater than or equal to the current block.
    /// - If there are failed blocks, retrieves them and sends them to the worker queue.
    /// - Marks handled failed blocks in the database.
    /// - Sends the current block to the worker queue and increments `current_block`.
    /// - If the latest block is not yet available, it waits before rechecking.
    ///
    /// # Shutdown Handling:
    /// - The loop will terminate if `self.finish_handle.is_shutdown_requested()` returns `true`.
    /// - If sending to the worker queue (`task_tx.send()`) fails, the loop exits early.
    ///
    /// # Blocking Behavior:
    /// - If no new block is available, it waits for `BLOCK_CHECK_INTERVAL` before retrying.
    async fn run(mut self) {
        let (task_tx, task_rx) = mpsc::channel(TASK_BUFFER_SIZE);
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(task_rx));

        for _ in 0..self.workers_count {
            let worker_task_rx = task_rx.clone();
            let finish_handle = self.finish_handle.clone();
            let rpc_url = self.rpc_url.clone();
            let channel = self.channel.clone();
            let snos = self.snos.clone();

            workers.push(task::spawn(Self::worker(
                worker_task_rx,
                finish_handle,
                rpc_url,
                channel,
                snos,
                self.db.clone(),
            )));
        }

        while !self.finish_handle.is_shutdown_requested() {
            match self.get_latest_block().await {
                Some(latest_block) if latest_block >= self.current_block => {
                    if let Ok(mut failed_blocks) = self.db.get_failed_blocks().await {
                        let block_ids: Vec<u32> = failed_blocks.iter().map(|(id, _)| *id).collect();
                        for (block_id, _) in failed_blocks.drain(..) {
                            if task_tx.send(block_id as u64).await.is_err() {
                                return;
                            }
                        }
                        self.db
                            .mark_failed_blocks_as_handled(&block_ids)
                            .await
                            .unwrap();
                    }
                    if task_tx.send(self.current_block).await.is_err() {
                        return;
                    }
                    self.current_block += 1;
                }
                _ => {
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

impl<S, DB> PollingBlockIngestorBuilder<S, DB> {
    pub fn new(rpc_url: Url, snos: S, db: DB, workers_count: usize) -> Self {
        Self {
            rpc_url,
            snos,
            start_block: None,
            channel: None,
            db,
            workers_count,
        }
    }
}

impl<S, DB> BlockIngestorBuilder for PollingBlockIngestorBuilder<S, DB>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Ingestor = PollingBlockIngestor<S, DB>;

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
            db: self.db,
            workers_count: self.workers_count,
        })
    }

    fn start_block(mut self, start_block: u64) -> Self {
        self.start_block = Some(start_block);
        self
    }

    fn channel(mut self, channel: Sender<BlockInfo>) -> Self {
        self.channel = Some(channel);
        self
    }
}

impl<S, DB> BlockIngestor for PollingBlockIngestor<S, DB>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
}

impl<S, DB> Daemon for PollingBlockIngestor<S, DB>
where
    S: AsRef<[u8]> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
