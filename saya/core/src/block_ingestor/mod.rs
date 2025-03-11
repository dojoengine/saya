use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use std::future::Future;
use tokio::sync::mpsc::Sender;

pub mod pie_generator;
mod polling;

pub use polling::{PollingBlockIngestor, PollingBlockIngestorBuilder};

use crate::{service::Daemon, storage::BlockStatus};

pub trait BlockIngestorBuilder {
    type Ingestor: BlockIngestor;

    fn build(self) -> Result<Self::Ingestor>;

    fn start_block(self, start_block: u64) -> Self;

    fn channel(self, channel: Sender<BlockInfo>) -> Self;
}

pub trait BlockIngestor: Daemon {}

pub trait BlockPieGenerator: Send + Sync {
    fn prove_block(
        &self,
        snos: &[u8],
        block_number: u64,
        rpc_url: &str,
    ) -> impl Future<Output = Result<CairoPie>> + Send;
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub number: u64,
    pub status: BlockStatus,
}
