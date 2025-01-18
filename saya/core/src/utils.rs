use std::time::Duration;

use anyhow::Result;
use bigdecimal::{
    num_bigint::{BigInt, Sign},
    BigDecimal,
};
use num_traits::ToPrimitive;
use starknet::{
    core::types::{ExecutionResult, StarknetError},
    providers::{Provider, ProviderError},
};
use starknet_types_core::felt::Felt;
use swiftness_stark::types::StarkProof;

// Ported from `saya` pre-rewrite.
pub fn calculate_output(proof: &StarkProof) -> Vec<Felt> {
    let output_segment = &proof.public_input.segments[2];
    let output_len = output_segment.stop_ptr - output_segment.begin_addr;
    let start = proof.public_input.main_page.len() - output_len.to_usize().unwrap();
    let end = proof.public_input.main_page.len();
    proof.public_input.main_page[start..end]
        .iter()
        .map(|cell| cell.value)
        .collect::<Vec<_>>()
}

pub fn felt_to_bigdecimal<F, D>(felt: F, decimals: D) -> BigDecimal
where
    F: AsRef<Felt>,
    D: Into<i64>,
{
    BigDecimal::new(
        BigInt::from_bytes_be(Sign::Plus, &felt.as_ref().to_bytes_be()),
        decimals.into(),
    )
}

pub async fn watch_tx<P>(provider: P, transaction_hash: Felt, poll_interval: Duration) -> Result<()>
where
    P: Provider,
{
    loop {
        match provider.get_transaction_receipt(transaction_hash).await {
            Ok(receipt) => match receipt.receipt.execution_result() {
                ExecutionResult::Succeeded => {
                    return Ok(());
                }
                ExecutionResult::Reverted { reason } => {
                    return Err(anyhow::anyhow!("transaction reverted: {}", reason));
                }
            },
            Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound)) => {}
            Err(err) => return Err(err.into()),
        }

        tokio::time::sleep(poll_interval).await;
    }
}
