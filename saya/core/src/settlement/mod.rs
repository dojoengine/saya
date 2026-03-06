use std::future::Future;

use anyhow::Result;
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{prover::TeeProof, service::Daemon};

/// Settlement backend builder for the TEE pipeline.
///
/// Receives [`TeeProof`] directly — all calldata fields are carried on the proof so no
/// `DataAvailabilityCursor` adapter is needed.
pub trait TeeSettlementBackendBuilder {
    type Backend: SettlementBackend;

    fn build(self) -> impl Future<Output = Result<Self::Backend>> + Send;

    fn proof_channel(self, proof_channel: Receiver<TeeProof>) -> Self;

    fn cursor_channel(self, cursor_channel: Sender<SettlementCursor>) -> Self;
}

pub trait SettlementBackend: Daemon {
    /// Gets the block number of the last block verified by the settlement layer.
    ///
    /// Returns a `Felt` since the genesis previous block value is `Felt::MAX`.
    fn get_block_number(&self) -> impl Future<Output = Result<Felt>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementCursor {
    /// Number/height of the rollup block that has been settled.
    pub block_number: u64,
    /// Settlement transaction hash.
    pub transaction_hash: Felt,
}
