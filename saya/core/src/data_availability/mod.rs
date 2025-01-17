use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

mod celestia;
pub use celestia::{CelestiaDataAvailabilityBackend, CelestiaDataAvailabilityBackendBuilder};

use crate::{prover::Proof, service::Daemon};

pub trait DataAvailabilityBackendBuilder {
    type Backend: DataAvailabilityBackend;

    fn build(self) -> Result<Self::Backend>;

    fn last_pointer(self, last_pointer: Option<DataAvailabilityPointer>) -> Self;

    fn proof_channel(self, proof_channel: Receiver<Proof>) -> Self;

    fn cursor_channel(self, cursor_channel: Sender<DataAvailabilityCursor>) -> Self;
}

pub trait DataAvailabilityBackend: Daemon {}

/// A data availability packet contains data being made available alongside a pointer to the
/// previous packet.
///
/// Note that such a design makes an implicit assumption that a full chain of available data can be
/// retrieved by following the pointers backward. This goes against the purpose of data availability
/// layers, which exist only to ensure certain pieces of data are published for a limited period of
/// time, *not* that such data would remain retrievable afterwards.
///
/// This issue shouldn't matter much during the proof of concept stage, but should definitely be
/// revisited before getting production-ready.
#[derive(Debug, Serialize, Deserialize)]
pub struct DataAvailabilityPacket {
    /// Pointer to the previous [`DataAvailabilityPacket`].
    pub prev: Option<DataAvailabilityPointer>,
    /// The content of the packet.
    pub content: DataAvailabilityContent,
}

// TODO: abstract over this to allow other DA backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataAvailabilityPointer {
    /// Celestia block height.
    pub height: u64,
    /// Celestia blob commitment.
    pub commitment: [u8; 32],
}

// TODO: abstract over this to allow other DA backends.
#[derive(Debug, Serialize, Deserialize)]
pub struct DataAvailabilityContent {
    /// State transition start block.
    pub from_block_number: u64,
    /// State transition end block. This should be the same as `from_block_number` if the transition
    /// involves only one block.
    pub to_block_number: u64,
    /// The STARK proof for the state transition.
    pub proof: StarkProof,
}

// TODO: abstract over this to allow other DA backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataAvailabilityCursor {
    /// State transition start block.
    pub from_block_number: u64,
    /// State transition end block. This should be the same as `from_block_number` if the transition
    /// involves only one block.
    pub to_block_number: u64,
    /// Pointer to location of data availability.
    pub pointer: DataAvailabilityPointer,
}
