use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use tokio::sync::mpsc::Sender;

mod polling;
pub use polling::{PollingBlockIngestor, PollingBlockIngestorBuilder};

use crate::service::Daemon;

pub trait BlockIngestorBuilder {
    type Ingestor: BlockIngestor;

    fn build(self) -> Result<Self::Ingestor>;

    fn start_block(self, start_block: u64) -> Self;

    fn channel(self, channel: Sender<NewBlock>) -> Self;
}

pub trait BlockIngestor: Daemon {}

#[derive(Debug)]
pub struct NewBlock {
    pub number: u64,
    pub pie: CairoPie,
    pub n_txs: u64,
}
