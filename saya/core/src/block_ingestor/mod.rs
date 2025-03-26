use anyhow::Result;

use tokio::sync::mpsc::Sender;

mod polling;
pub use polling::{PollingBlockIngestor, PollingBlockIngestorBuilder};

mod sharding;
use crate::{service::Daemon, storage::BlockStatus};
pub use sharding::polling::{ShardingIngestor, ShardingIngestorBuilder};

pub trait BlockIngestorBuilder {
    type Ingestor: BlockIngestor;

    fn build(self) -> Result<Self::Ingestor>;

    fn start_block(self, start_block: u64) -> Self;

    fn channel(self, channel: Sender<BlockInfo>) -> Self;
}

pub trait BlockIngestor: Daemon {}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub number: u64,
    pub status: BlockStatus,
}
