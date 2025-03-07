use anyhow::Result;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    data_availability::{
        DataAvailabilityPacketContext, DataAvailabilityPayload, PersistentPacket, SovereignPacket,
    },
    service::Daemon,
};

mod atlantic;
pub use atlantic::{
    AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder, AtlanticSnosProver,
    AtlanticSnosProverBuilder,
};

mod mock;
pub use mock::{MockLayoutBridgeProver, MockLayoutBridgeProverBuilder};
pub mod trace;
pub use trace::LayoutBridgeTraceGenerator;
mod recursive;
pub use atlantic::compress_pie;
pub use atlantic::AtlanticClient;
pub use recursive::{RecursiveProver, RecursiveProverBuilder};

pub mod error;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnosProof<P> {
    pub block_number: u64,
    pub proof: P,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecursiveProof {
    pub block_number: u64,
    pub snos_output: Vec<Felt>,
    pub layout_bridge_proof: StarkProof,
}

impl DataAvailabilityPayload for SnosProof<StarkProof> {
    type Packet = SovereignPacket;

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn into_packet(self, ctx: DataAvailabilityPacketContext) -> Self::Packet {
        SovereignPacket {
            prev: ctx.prev,
            proof: self,
        }
    }
}

impl DataAvailabilityPayload for RecursiveProof {
    type Packet = PersistentPacket;

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn into_packet(self, _ctx: DataAvailabilityPacketContext) -> Self::Packet {
        PersistentPacket
    }
}
