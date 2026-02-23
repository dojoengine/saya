pub mod env;

use std::time::Duration;

use anyhow::Result;
use piltover::{AppchainContractReader, ProgramInfo};
use starknet::{
    core::types::{BlockId, BlockTag, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
};

/// Decoded appchain state returned by the piltover `get_state` entry point.
#[derive(Debug)]
pub struct AppchainState {
    pub state_root: Felt,
    pub block_number: Felt,
    pub block_hash: Felt,
}

/// Query the piltover contract for the current settled chain state.
pub async fn get_settlement_state(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<AppchainState> {
    let (state_root, block_number, block_hash) =
        AppchainContractReader::new(piltover_address, provider)
            .with_block(BlockId::Tag(BlockTag::Latest))
            .get_state()
            .call()
            .await?;

    Ok(AppchainState {
        state_root,
        block_number,
        block_hash,
    })
}

/// Query the piltover contract for the configured program info.
pub async fn get_program_info(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<ProgramInfo> {
    Ok(AppchainContractReader::new(piltover_address, provider)
        .with_block(BlockId::Tag(BlockTag::Latest))
        .get_program_info()
        .call()
        .await?)
}

/// Query the piltover contract for the configured fact registry address.
pub async fn get_facts_registry(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<Felt> {
    let address = AppchainContractReader::new(piltover_address, provider)
        .with_block(BlockId::Tag(BlockTag::Latest))
        .get_facts_registry()
        .call()
        .await?;

    Ok(Felt::from(address))
}

/// Poll the piltover `get_state` entry point until `block_number >= target_block`
/// or the deadline is exceeded.
///
/// The initial (genesis) state uses `Felt::MAX` as a sentinel meaning no block has
/// been settled yet. This function treats that as "not yet at target".
pub async fn wait_for_settlement(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
    target_block: u64,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<AppchainState> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let state = get_settlement_state(provider, piltover_address).await?;

        // Felt::MAX is the genesis sentinel — no block settled yet.
        if state.block_number != Felt::MAX {
            let settled: u64 = state.block_number.try_into()?;
            if settled >= target_block {
                return Ok(state);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timeout waiting for settlement of block {}", target_block);
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Build a JSON-RPC provider from a URL string.
pub fn provider(url: &str) -> JsonRpcClient<HttpTransport> {
    JsonRpcClient::new(HttpTransport::new(
        url.parse::<url::Url>().expect("invalid RPC URL"),
    ))
}
