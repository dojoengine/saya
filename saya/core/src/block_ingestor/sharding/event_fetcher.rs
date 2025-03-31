use std::sync::Arc;

use starknet::{
    core::types::{BlockId, Event},
    providers::{
        jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError::StarknetError, Url,
    },
};
use starknet_crypto::Felt;
use tokio::time::sleep;

pub async fn look_for_event(
    contract_address: Felt,
    block_number: u64,
    rpc_url: Url,
    event_hash: Felt,
) -> bool {
    let provider: Arc<JsonRpcClient<HttpTransport>> =
        Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url)));
    let events = get_events_for_blocks(block_number, provider.clone()).await;
    for event in events {
        if event.from_address == contract_address && event.keys[0] == event_hash {
            println!("Event found: {:?} at block {}", event, block_number);
            return true;
        }
    }
    false
}

pub async fn get_events_for_blocks(
    block_number: u64,
    provider: Arc<JsonRpcClient<HttpTransport>>,
) -> Vec<Event> {
    let block = BlockId::Number(block_number);
    loop {
        match provider.get_block_with_receipts(block).await {
            Ok(block_with_receipts) => {
                let events = block_with_receipts
                    .transactions()
                    .iter()
                    .flat_map(|tx| tx.receipt.events().to_vec())
                    .collect::<Vec<Event>>();
                return events;
            }
            Err(StarknetError(starknet::core::types::StarknetError::BlockNotFound)) => {
                println!("Block {} not found yet. Retrying...", block_number);
                sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                println!("Error fetching block {}: {:?}", block_number, e);
                return vec![];
            }
        }
    }
}
