use starknet::accounts::single_owner::SignError;
use starknet::accounts::AccountError;
use starknet::signers::local_wallet::SignError as LocalSignError;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error("Block {0:?} not found.")]
    BlockNotFound(u32),
    #[error(transparent)]
    Snos(#[from] starknet_os::error::SnOsError),
    #[error(transparent)]
    ProveBlock(#[from] prove_block::ProveBlockError),
    #[error("Invalid chain_id ")]
    InvalidChainId,
    #[error("{0}")]
    TimeoutError(String),
    #[error("{0}")]
    TransactionRejected(String),
    #[error("{0}")]
    TransactionFailed(String),
    #[error("{0}")]
    TryFromStrError(String),
    #[error("Atlantic server is not alive")]
    ServerNotAliveError,
    #[error(transparent)]
    SerdeFeltError(#[from] serde_felt::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    SerdeJsonError(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    UrlParseError(#[from] url::ParseError),
    #[error(transparent)]
    SharpError(#[from] atlantic_client::error::AtlanticSdkError),
    #[error(transparent)]
    RequestError(#[from] reqwest::Error),
    #[error(transparent)]
    StarknetProviderError(#[from] starknet::providers::ProviderError),
    #[error(transparent)]
    StarknetTransactionError(#[from] AccountError<SignError<LocalSignError>>),
}
