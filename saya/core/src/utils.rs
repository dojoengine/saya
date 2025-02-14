use std::time::Duration;

use anyhow::Result;
use bigdecimal::{
    num_bigint::{BigInt, Sign},
    BigDecimal,
};
use cairo_vm::{program_hash::compute_program_hash_chain, vm::runners::cairo_pie::CairoPie};
use num_traits::ToPrimitive;
use starknet::{
    core::types::{Call, ExecutionResult, StarknetError, TransactionReceiptWithBlockInfo},
    providers::{Provider, ProviderError},
};
use starknet_types_core::felt::Felt;
use swiftness_air::types::SegmentInfo;
use swiftness_stark::types::StarkProof;

const STARKNET_TX_CALLDATA_LIMIT: usize = 5_000;

/// 3 extra field elements are needed to add a call:
///
/// - callee contract address
/// - entrypoint selector
/// - calldata length prefix
const ACCOUNT_CALL_OVERHEAD: usize = 3;

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

pub async fn watch_tx<P>(
    provider: P,
    transaction_hash: Felt,
    poll_interval: Duration,
) -> Result<TransactionReceiptWithBlockInfo>
where
    P: Provider,
{
    loop {
        match provider.get_transaction_receipt(transaction_hash).await {
            Ok(receipt) => match receipt.receipt.execution_result() {
                ExecutionResult::Succeeded => {
                    return Ok(receipt);
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

pub fn split_calls(calls: Vec<Call>) -> Vec<Vec<Call>> {
    let mut chunks = vec![];

    let mut iter = calls.into_iter().peekable();

    while iter.peek().is_some() {
        let mut chunk = vec![];

        // 1 slot is always used for calls length prefix
        let mut chunk_size = 1;

        while let Some(call) = iter.next_if(|next_call| {
            chunk_size + next_call.calldata.len() + ACCOUNT_CALL_OVERHEAD
                <= STARKNET_TX_CALLDATA_LIMIT
        }) {
            chunk_size += call.calldata.len() + ACCOUNT_CALL_OVERHEAD;
            chunk.push(call);
        }

        chunks.push(chunk);
    }

    chunks
}

/// Computes the program hash from a `CairoPie` instance, mostly used for
/// testing to avoid extracting the program hash from the SHARP bootloader.
/// (which also extracts the program hash from the PIE)
pub fn compute_program_hash_from_pie(pie: &CairoPie) -> Result<Felt> {
    let hash = compute_program_hash_chain(&pie.metadata.program, 0)?;
    let bytes = hash.to_bytes_be();
    Ok(Felt::from_bytes_be(&bytes))
}

/// Extracts the output of a program from a `CairoPie`.
///
/// This output is the one that is returned by the prover at the end
/// of the `public_input`.
pub fn extract_pie_output(pie: &CairoPie) -> Vec<Felt> {
    let output_segment_index = 2_usize;
    let output_segment = prove_block::get_memory_segment(pie, output_segment_index);
    let output: Vec<Felt> = output_segment
        .iter()
        .map(|(_key, value)| value.get_int().unwrap())
        .collect::<Vec<_>>();
    output
}

/// This proof is mocked but calling `calculate_output` on it correctly yields
/// the expected output.
///
/// This spaghetti is needed because `StarkProof` does not implement `Default`.
pub fn stark_proof_mock(output: &[Felt]) -> StarkProof {
    StarkProof {
        config: swiftness::config::StarkConfig {
            traces: swiftness_air::trace::config::Config {
                original: default_table_commitment_config(),
                interaction: default_table_commitment_config(),
            },
            composition: default_table_commitment_config(),
            fri: swiftness_fri::config::Config {
                log_input_size: Default::default(),
                n_layers: Default::default(),
                inner_layers: Default::default(),
                fri_step_sizes: Default::default(),
                log_last_layer_degree_bound: Default::default(),
            },
            proof_of_work: swiftness_pow::config::Config {
                n_bits: Default::default(),
            },
            log_trace_domain_size: Default::default(),
            n_queries: Default::default(),
            log_n_cosets: Default::default(),
            n_verifier_friendly_commitment_layers: Default::default(),
        },
        public_input: swiftness_air::public_memory::PublicInput {
            log_n_steps: Default::default(),
            range_check_min: Default::default(),
            range_check_max: Default::default(),
            layout: Default::default(),
            dynamic_params: Default::default(),
            segments: vec![
                SegmentInfo {
                    begin_addr: Default::default(),
                    stop_ptr: Default::default(),
                },
                SegmentInfo {
                    begin_addr: Default::default(),
                    stop_ptr: Default::default(),
                },
                SegmentInfo {
                    begin_addr: Felt::ZERO,
                    stop_ptr: Felt::from(output.len()),
                },
            ],
            padding_addr: Default::default(),
            padding_value: Default::default(),
            main_page: swiftness_air::types::Page(
                output
                    .iter()
                    .map(|value| swiftness_air::types::AddrValue {
                        address: Default::default(),
                        value: *value,
                    })
                    .collect(),
            ),
            continuous_page_headers: Default::default(),
        },
        unsent_commitment: swiftness::types::StarkUnsentCommitment {
            traces: swiftness_air::trace::UnsentCommitment {
                original: Default::default(),
                interaction: Default::default(),
            },
            composition: Default::default(),
            oods_values: Default::default(),
            fri: swiftness_fri::types::UnsentCommitment {
                inner_layers: Default::default(),
                last_layer_coefficients: Default::default(),
            },
            proof_of_work: swiftness_pow::pow::UnsentCommitment {
                nonce: Default::default(),
            },
        },
        witness: swiftness_stark::types::StarkWitness {
            traces_decommitment: swiftness_air::trace::Decommitment {
                original: swiftness_commitment::table::types::Decommitment {
                    values: Default::default(),
                },
                interaction: swiftness_commitment::table::types::Decommitment {
                    values: Default::default(),
                },
            },
            traces_witness: swiftness_air::trace::Witness {
                original: swiftness_commitment::table::types::Witness {
                    vector: swiftness_commitment::vector::types::Witness {
                        authentications: Default::default(),
                    },
                },
                interaction: swiftness_commitment::table::types::Witness {
                    vector: swiftness_commitment::vector::types::Witness {
                        authentications: Default::default(),
                    },
                },
            },
            composition_decommitment: swiftness_commitment::table::types::Decommitment {
                values: Default::default(),
            },
            composition_witness: swiftness_commitment::table::types::Witness {
                vector: swiftness_commitment::vector::types::Witness {
                    authentications: Default::default(),
                },
            },
            fri_witness: swiftness_fri::types::Witness {
                layers: Default::default(),
            },
        },
    }
}

fn default_table_commitment_config() -> swiftness_commitment::table::config::Config {
    swiftness_commitment::table::config::Config {
        n_columns: Default::default(),
        vector: swiftness_commitment::vector::config::Config {
            height: Default::default(),
            n_verifier_friendly_commitment_layers: Default::default(),
        },
    }
}
