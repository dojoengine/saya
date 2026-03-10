//! TEE (Trusted Execution Environment) types and pipeline stages.
//!
//! Pipeline flow:
//!   `Vec<BlockInfo>` → [`TeeAttestor`] → [`TeeAttestation`]
//!                    → [`OffchainTeeVerifier`] → [`TeeTrace`]
//!                    → (TEE Prover — see [`crate::prover::tee`])
//!                    → [`crate::prover::tee::TeeProof`]

use starknet_types_core::felt::Felt;

use crate::{block_ingestor::BlockInfo, prover::HasBlockNumber};

/// Attestation data fetched from a Katana rollup node for a batch of blocks.
///
/// The `blocks` are threaded through the entire TEE pipeline so that downstream stages
/// (particularly the settlement adapter) retain access to the original [`BlockInfo`] range
/// without needing an extra DB round-trip.
#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// The ordered batch of blocks covered by this attestation.
    pub blocks: Vec<BlockInfo>,
    /// Raw attestation bytes returned by Katana (hex-encoded AMD SEV-SNP quote).
    pub quote: String,
    pub prev_state_root: String,
    pub state_root: String,
    pub prev_block_hash: String,
    pub block_hash: String,
    pub prev_block_number: Felt,
    pub block_number: Felt,
}

/// Execution trace produced by the offchain TEE verifier from a [`TeeAttestation`].
///
/// This trace is the input to the TEE prover service.
#[derive(Debug, Clone)]
pub struct TeeTrace {
    /// The ordered batch of blocks covered by this trace.
    pub blocks: Vec<BlockInfo>,
    /// Raw trace bytes to be fed into the TEE prover.
    ///
    /// TODO: define concrete trace format once the verifier API is stable.
    pub data: Vec<u8>,
}

impl HasBlockNumber for TeeAttestation {
    /// Returns the block number of the last block in the batch — used for pipeline ordering.
    fn block_number(&self) -> u64 {
        self.blocks
            .last()
            .expect("non-empty attestation batch")
            .number
    }
}

impl HasBlockNumber for TeeTrace {
    /// Returns the block number of the last block in the batch — used for pipeline ordering.
    fn block_number(&self) -> u64 {
        self.blocks.last().expect("non-empty trace batch").number
    }
}
