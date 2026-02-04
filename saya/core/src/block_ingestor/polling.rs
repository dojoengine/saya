use std::{sync::Arc, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use generate_pie::{
    generate_pie,
    types::{ChainConfig, OsHintsConfiguration},
};
use log::{debug, error, info, trace};
use starknet::{
    core::types::BlockId,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
};
use starknet_api::{contract_address, core::ChainId};
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

const BLOCK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TASK_BUFFER_SIZE: usize = 4;
const MAX_RETRIES: usize = 3;
const KATANA_DEFAULT_TOKEN_ADDRESS: &str =
    "0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea";
/// A block ingestor which collects new blocks by polling a Starknet RPC endpoint.
#[derive(Debug)]
pub struct PollingBlockIngestor<DB> {
    rpc_url: Url,
    current_block: u64,
    channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
    chain_id: ChainId,
    os_hints_config: OsHintsConfiguration,
}

#[derive(Debug)]
pub struct PollingBlockIngestorBuilder<DB> {
    rpc_url: Url,
    start_block: Option<u64>,
    channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
    chain_id: ChainId,
    os_hints_config: OsHintsConfiguration,
}

impl<DB> PollingBlockIngestor<DB>
where
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
                error!(error:? = err; "Failed to fetch latest block");
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
        db: DB,
        os_hints_config: OsHintsConfiguration,
        chain_id: ChainId,
    ) where
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
            let state_update = &JsonRpcClient::new(HttpTransport::new(rpc_url.clone()))
                .get_state_update(BlockId::Number(block_number))
                .await
                .unwrap();
            let state_update = match state_update {
                starknet::core::types::MaybePreConfirmedStateUpdate::Update(state_update) => {
                    state_update
                }
                //TODO: handle this case properly
                starknet::core::types::MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                    panic!("PreConfirmedStateUpdate not supported")
                }
            };

            db.add_state_update(block_number.try_into().unwrap(), state_update.clone())
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
                            state_update: Some(state_update.clone()),
                        };
                        trace!(block_number; "Pie generated");

                        if channel.send(new_block).await.is_err() {
                            error!(block_number; "Failed to send block");
                        }
                        continue;
                    }
                    Err(err) => {
                        error!(block_number,error:% = err; "Failed to parse pie")
                    }
                },
                Err(err) => {
                    // Not found in db, we continue
                    // Should we log this error, as this is kinda intended to not found pie on first iteration?
                    trace!( block_number, error:% =err; "Pie not found in db");
                }
            }

            let pie_input = generate_pie::types::PieGenerationInput {
                rpc_url: rpc_url.to_string(),
                blocks: vec![block_number],
                versioned_constants: None,
                chain_config: ChainConfig {
                    chain_id: chain_id.clone(),
                    strk_fee_token_address: contract_address!(KATANA_DEFAULT_TOKEN_ADDRESS),
                    is_l3: true,
                    eth_fee_token_address: contract_address!(KATANA_DEFAULT_TOKEN_ADDRESS),
                },
                layout: cairo_vm::types::layout_name::LayoutName::all_cairo,
                os_hints_config: os_hints_config.clone(),
                output_path: None,
            };

            let pie = generate_pie(pie_input).await.unwrap().output.cairo_pie;

            if finish_handle.is_shutdown_requested() {
                break;
            }

            let new_block = BlockInfo {
                number: block_number,
                status: BlockStatus::SnosPieGenerated,
                state_update: Some(state_update.clone()),
            };

            let pie_bytes = compress_pie(pie.clone()).await.unwrap();
            let block_number = block_number.try_into().unwrap();

            db.add_pie(block_number, pie_bytes.clone(), Step::Snos)
                .await
                .unwrap();

            info!(block_number; "Pie generated for block");

            if channel.send(new_block).await.is_err() {
                error!(block_number; "Failed to send block");
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

            workers.push(task::spawn(Self::worker(
                worker_task_rx,
                finish_handle,
                rpc_url,
                channel,
                self.db.clone(),
                self.os_hints_config.clone(),
                self.chain_id.clone(),
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

impl<DB> PollingBlockIngestorBuilder<DB> {
    pub fn new(
        rpc_url: Url,
        db: DB,
        workers_count: usize,
        os_hints_config: OsHintsConfiguration,
        chain_id: ChainId,
    ) -> Self {
        Self {
            rpc_url,
            start_block: None,
            channel: None,
            db,
            workers_count,
            chain_id,
            os_hints_config,
        }
    }
}

impl<DB> BlockIngestorBuilder for PollingBlockIngestorBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Ingestor = PollingBlockIngestor<DB>;

    fn build(self) -> Result<Self::Ingestor> {
        Ok(PollingBlockIngestor {
            rpc_url: self.rpc_url,
            db: self.db,
            current_block: self
                .start_block
                .ok_or_else(|| anyhow::anyhow!("`start_block` not set"))?,
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("`channel` not set"))?,
            finish_handle: FinishHandle::new(),
            workers_count: self.workers_count,
            chain_id: self.chain_id,
            os_hints_config: self.os_hints_config,
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

impl<DB> BlockIngestor for PollingBlockIngestor<DB> where
    DB: PersistantStorage + Send + Sync + Clone + 'static
{
}

impl<DB> Daemon for PollingBlockIngestor<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
