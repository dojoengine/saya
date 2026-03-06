use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProverError {
    #[error("prover error: {0}")]
    Prover(String),
    #[error("Shutdown signal received")]
    Shutdown,
    #[error("Block fail in Prover: {0}")]
    BlockFail(String),
    #[error("{0}")]
    MetadataFetch(String),
    #[error("{0}")]
    ProofParse(String),
}
