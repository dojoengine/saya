//! TEE (Trusted Execution Environment) types and pipeline stages.
//!
//! Pipeline flow:
//!   [`BlockInfo`] → [`TeeAttestor`] → [`TeeAttestation`]
//!                → [`OffchainTeeVerifier`] → [`TeeTrace`]
//!                → (TEE Prover — see [`crate::prover::tee`])
//!                → [`crate::prover::tee::TeeProof`]

mod attestor;
mod verifier;

pub use attestor::{TeeAttestor, TeeAttestorBuilder};
pub use verifier::{OffchainTeeVerifier, OffchainTeeVerifierBuilder};

use crate::{block_ingestor::BlockInfo, prover::HasBlockNumber};

/// Attestation data fetched from a Katana rollup node for a specific block.
///
/// The `block_info` is threaded through the entire TEE pipeline so that downstream stages
/// (particularly the settlement adapter) retain access to the original [`BlockInfo`] without
/// needing an extra DB round-trip.
#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// Original block info, carried through the pipeline.
    pub block_info: BlockInfo,
    /// Raw attestation bytes returned by Katana.
    ///
    /// TODO: replace with a concrete attestation struct once the Katana TEE API is stable.
    pub raw: Vec<u8>,
}

/// Execution trace produced by the offchain TEE verifier from a [`TeeAttestation`].
///
/// This trace is the input to the TEE prover service.
#[derive(Debug, Clone)]
pub struct TeeTrace {
    /// Original block info, carried through the pipeline.
    pub block_info: BlockInfo,
    /// Raw trace bytes to be fed into the TEE prover.
    ///
    /// TODO: define concrete trace format once the verifier API is stable.
    pub data: Vec<u8>,
}

impl HasBlockNumber for TeeAttestation {
    fn block_number(&self) -> u64 {
        self.block_info.number
    }
}

impl HasBlockNumber for TeeTrace {
    fn block_number(&self) -> u64 {
        self.block_info.number
    }
}
