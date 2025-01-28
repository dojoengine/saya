use anyhow::Result;
use serde::Serialize;
use starknet_types_core::felt::Felt;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{data_availability::DataAvailabilityPayload, service::Daemon};

mod atlantic;
pub use atlantic::{
    AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder, AtlanticSnosProver,
    AtlanticSnosProverBuilder,
};

mod recursive;
pub use recursive::{RecursiveProver, RecursiveProverBuilder};

pub trait ProverBuilder {
    type Prover: Prover;

    fn build(self) -> Result<Self::Prover>;

    fn statement_channel(
        self,
        block_channel: Receiver<<Self::Prover as Prover>::Statement>,
    ) -> Self;

    fn proof_channel(self, proof_channel: Sender<<Self::Prover as Prover>::Proof>) -> Self;
}

pub trait Prover: Daemon {
    type Statement;
    type Proof;
}

#[derive(Debug, Clone, Serialize)]
pub struct SnosProof<P> {
    pub block_number: u64,
    pub proof: P,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecursiveProof {
    pub block_number: u64,
    pub snos_output: Vec<Felt>,
    pub layout_bridge_proof: StarkProof,
}

impl<P> DataAvailabilityPayload for SnosProof<P>
where
    P: Serialize + Clone + Send,
{
    fn block_number(&self) -> u64 {
        self.block_number
    }
}

impl DataAvailabilityPayload for RecursiveProof {
    fn block_number(&self) -> u64 {
        self.block_number
    }
}
