use anyhow::Result;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{block_ingestor::NewBlock, service::Daemon};

mod atlantic;
pub use atlantic::{AtlanticProver, AtlanticProverBuilder};

pub trait ProverBuilder {
    type Prover: Prover;

    fn build(self) -> Result<Self::Prover>;

    fn block_channel(self, block_channel: Receiver<NewBlock>) -> Self;

    fn proof_channel(self, proof_channel: Sender<Proof>) -> Self;
}

pub trait Prover: Daemon {}

#[derive(Debug)]
pub struct Proof {
    pub block_number: u64,
    pub proof: StarkProof,
}
