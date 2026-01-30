use anyhow::Result;

use starknet::core::types::StateUpdate;
use tokio::sync::mpsc::Sender;

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

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub number: u64,
    pub status: BlockStatus,
    pub state_update: Option<StateUpdate>,
}

#[derive(Debug, Clone)]
pub struct BlobPointer {
    pub height: u64,
    pub commitment: [u8; 32],
    pub namespace: String,
}
