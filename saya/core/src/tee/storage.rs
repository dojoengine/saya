//! TEE persistence layer.
//!
//! Defines the [`TeeStorage`] trait and related types for persisting batch state
//! across TEE pipeline stages (attestation → proof → settlement).

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Unique identifier for a TEE batch in persistent storage.
pub type BatchId = i64;

/// Status of a batch in the TEE pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeeBatchStatus {
    /// Waiting to be attested.
    PendingAttestation,
    /// Attestation completed; stored in DB.
    Attested,
    /// SP1 proof generated; stored in DB.
    Proved,
    /// Proof submitted to settlement; awaiting confirmation.
    SettlementPending,
    /// Proof settled on-chain; batch complete.
    Settled,
    /// Batch failed after exhausting retries.
    Failed,
}

impl std::fmt::Display for TeeBatchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeeBatchStatus::PendingAttestation => write!(f, "pending_attestation"),
            TeeBatchStatus::Attested => write!(f, "attested"),
            TeeBatchStatus::Proved => write!(f, "proved"),
            TeeBatchStatus::SettlementPending => write!(f, "settlement_pending"),
            TeeBatchStatus::Settled => write!(f, "settled"),
            TeeBatchStatus::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for TeeBatchStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending_attestation" => Ok(TeeBatchStatus::PendingAttestation),
            "attested" => Ok(TeeBatchStatus::Attested),
            "proved" => Ok(TeeBatchStatus::Proved),
            "settlement_pending" => Ok(TeeBatchStatus::SettlementPending),
            "settled" => Ok(TeeBatchStatus::Settled),
            "failed" => Ok(TeeBatchStatus::Failed),
            other => Err(format!("unknown status: {}", other)),
        }
    }
}

/// Attestation metadata ready to be stored or loaded from persistent storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAttestation {
    /// Raw attestation response from Katana (hex-encoded AMD SEV-SNP quote).
    pub quote: String,
    pub prev_state_root: String,
    pub state_root: String,
    pub prev_block_hash: String,
    pub block_hash: String,
    pub prev_block_number: String,
    pub block_number: String,
    /// Poseidon commitment over all L1↔L2 messages.
    pub messages_commitment: String,
    /// JSON-serialized L2→L1 messages.
    pub l2_to_l1_messages: String,
    /// JSON-serialized L1→L2 messages.
    pub l1_to_l2_messages: String,
}

/// Batch metadata loaded from persistent storage for recovery.
#[derive(Debug, Clone)]
pub struct IncompleteBatch {
    /// Database row ID for this batch.
    pub batch_id: BatchId,
    /// First block number in the batch.
    pub first_block: u64,
    /// Last block number in the batch.
    pub last_block: u64,
    /// Current status in the pipeline.
    pub status: TeeBatchStatus,
    /// Stored attestation (if status >= Attested).
    pub attestation: Option<StoredAttestation>,
    /// Stored proof bytes (if status >= Proved).
    pub proof: Option<Vec<u8>>,
    /// Stored settlement transaction hash (if status >= SettlementPending).
    pub settlement_tx_hash: Option<String>,
    /// Number of times this batch has been retried.
    pub retry_count: u32,
}

/// Core trait for TEE batch persistence.
///
/// Implementations must ensure all operations are atomic per-batch (e.g., status updates
/// are paired with the corresponding data write in a single transaction).
pub trait TeeStorage: Send + Sync + 'static {
    /// Create a new batch and return its ID.
    fn create_batch(
        &self,
        first_block: u64,
        last_block: u64,
    ) -> impl std::future::Future<Output = Result<BatchId>> + Send;

    /// Update the batch status.
    fn set_batch_status(
        &self,
        id: BatchId,
        status: TeeBatchStatus,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Increment the retry count for a batch and return the new count.
    fn increment_retry_count(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<u32>> + Send;

    /// Save the raw attestation response (must be paired with set_batch_status(Attested) in a transaction).
    fn save_attestation(
        &self,
        id: BatchId,
        stored_data: &StoredAttestation,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Load the stored attestation for a batch.
    fn load_attestation(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<StoredAttestation>>> + Send;

    /// Save the SP1 proof bytes (must be paired with set_batch_status(Proved) in a transaction).
    fn save_proof(
        &self,
        id: BatchId,
        proof_bytes: &[u8],
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Load the stored proof for a batch.
    fn load_proof(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Save the settlement transaction hash (must be paired with set_batch_status(SettlementPending) in a transaction).
    fn save_settlement_tx(
        &self,
        id: BatchId,
        tx_hash: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Load the settlement transaction hash for a batch.
    fn get_settlement_tx(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send;

    /// Mark a settlement transaction as confirmed on-chain (pairs with set_batch_status(Settled)).
    fn confirm_settlement_tx(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get all batches that are not yet settled (status != Settled && status != Failed).
    fn get_incomplete_batches(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<IncompleteBatch>>> + Send;

    /// Get the last block number that was successfully settled on-chain (or None if no batches settled).
    fn get_last_settled_block(
        &self,
    ) -> impl std::future::Future<Output = Result<Option<u64>>> + Send;
}
