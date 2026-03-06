use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{block_ingestor::BlockInfo, service::Daemon};

pub mod tee;
pub use tee::{TeeProof, TeeProver, TeeProverBuilder};

mod block_orderer;
pub use block_orderer::{BlockOrderer, BlockOrdererBuilder};

mod recursive;
pub use recursive::{PipelineChain, PipelineChainBuilder};

pub mod error;

/// Implemented by pipeline items that carry a block number.
pub trait HasBlockNumber {
    fn block_number(&self) -> u64;
}

impl HasBlockNumber for BlockInfo {
    fn block_number(&self) -> u64 {
        self.number
    }
}

pub trait PipelineStageBuilder {
    type Stage: PipelineStage;

    fn build(self) -> Result<Self::Stage>;

    fn input_channel(self, input_channel: Receiver<<Self::Stage as PipelineStage>::Input>) -> Self;

    fn output_channel(self, output_channel: Sender<<Self::Stage as PipelineStage>::Output>)
        -> Self;

    fn start_block(self, _start_block: u64) -> Self
    where
        Self: Sized,
    {
        self
    }
}

pub trait PipelineStage: Daemon {
    type Input: Send + 'static;
    type Output: Send + 'static;
}
