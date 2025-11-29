use std::{future::Future, time::Duration};

use anyhow::Result;
use bigdecimal::{
    num_bigint::{BigInt, Sign},
    BigDecimal,
};
use cairo_vm::{
    program_hash::compute_program_hash_chain, types::relocatable::MaybeRelocatable,
    vm::runners::cairo_pie::CairoPie,
};
use integrity::Felt;
use log::debug;
use num_traits::ToPrimitive;
use starknet::{
    core::types::{Call, ExecutionResult, StarknetError, TransactionReceiptWithBlockInfo},
    providers::{Provider, ProviderError},
};
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

pub fn felt_to_bigdecimal<D>(felt: Felt, decimals: D) -> BigDecimal
where
    D: Into<i64>,
{
    BigDecimal::new(
        BigInt::from_bytes_be(Sign::Plus, &felt.to_bytes_be()),
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
    let output_segment = get_memory_segment(pie, output_segment_index);
    let output: Vec<Felt> = output_segment
        .iter()
        .map(|(_key, value)| value.get_int().unwrap())
        .collect::<Vec<_>>();
    output
}

pub fn get_memory_segment(pie: &CairoPie, index: usize) -> Vec<(usize, &MaybeRelocatable)> {
    let mut segment = pie
        .memory
        .0
        .iter()
        .filter_map(|((segment_index, offset), value)| {
            (*segment_index == index).then_some((*offset, value))
        })
        .collect::<Vec<_>>();
    segment.sort_by(|(offset1, _), (offset2, _)| offset1.cmp(offset2));
    segment
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

pub async fn retry_with_backoff<F, Fut, T, E>(
    operation: F,
    label: &str,
    max_attempts: u32,
    base_delay: Duration,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempts = 0;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                attempts += 1;
                if attempts >= max_attempts {
                    return Err(e);
                }
                let delay = base_delay * attempts;
                debug!(
                    "Operation {} failed on attempt {}/{}: {}. Retrying after {:?}...",
                    label, attempts, max_attempts, e, delay
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct MessageToL1 {
    pub from_address: Felt,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct MessageToL2 {
    pub from_address: Felt,
    pub to_address: Felt,
    pub nonce: Felt,
    pub selector: Felt,
    pub payload: Vec<Felt>,
}

/// The blockifier doesn't expose the trait to parse from felt the OsCommonOutput...
/// <https://github.com/starkware-libs/sequencer/blob/7193cc2247daefb08ff4462ea69e6c8b3a0ea4c5/crates/starknet_os/src/io/os_output.rs#L273>
///
/// So for now, we parse manually the messages from the program output.
pub fn extract_messages_from_program_output(
    program_output: &mut impl Iterator<Item = Felt>,
) -> (Vec<MessageToL1>, Vec<MessageToL2>) {
    // Index 18 starts the message to L1 segment length.
    // Then we need to parse each message to L1, which will then be followed by the message to L2 segment length.
    program_output.nth(17);

    let mut messages_to_l1 = vec![];
    let mut messages_to_l2 = vec![];

    let message_to_l1_segment_length = program_output.next().unwrap();
    dbg!(&message_to_l1_segment_length);

    if message_to_l1_segment_length.to_usize().unwrap() > 0 {
        let mut message_to_l1_cursor = 0;

        while message_to_l1_cursor < message_to_l1_segment_length.to_usize().unwrap() {
            let from_address = program_output.next().unwrap();
            let to_address = program_output.next().unwrap();
            message_to_l1_cursor += 2;

            let payload_len = program_output.next().unwrap().to_usize().unwrap();
            message_to_l1_cursor += 1;

            let payload = program_output
                .take(payload_len.to_usize().unwrap())
                .collect();

            message_to_l1_cursor += payload_len.to_usize().unwrap();

            messages_to_l1.push(MessageToL1 {
                from_address,
                to_address,
                payload,
            });
        }
    }

    let message_to_l2_segment_length = program_output.next().unwrap();
    dbg!(&message_to_l2_segment_length);

    let mut message_to_l2_cursor = 0;

    while message_to_l2_cursor < message_to_l2_segment_length.to_usize().unwrap() {
        let from_address = program_output.next().unwrap();
        let to_address = program_output.next().unwrap();
        let nonce = program_output.next().unwrap();
        let selector = program_output.next().unwrap();
        message_to_l2_cursor += 4;

        let payload_len = program_output.next().unwrap().to_usize().unwrap();
        message_to_l2_cursor += 1;

        let payload = program_output
            .take(payload_len.to_usize().unwrap())
            .collect();
        message_to_l2_cursor += payload_len.to_usize().unwrap();

        messages_to_l2.push(MessageToL2 {
            from_address,
            to_address,
            nonce,
            selector,
            payload,
        });
    }

    (messages_to_l1, messages_to_l2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet::macros::felt;

    #[test]
    fn test_extract_messages_only_messages_to_l1() {
        let program_output: Vec<Felt> = vec![
            felt!("0x1"),
            felt!("0x0"),
            felt!("0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8"),
            felt!("0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07"),
            felt!("0x0"),
            felt!("0x1"),
            felt!("0x0"),
            felt!("0x10e5341a417427d140af8f5def7d2cc687d84591ff8ec241623c590b5ca8c80"),
            felt!("0xe9955bc27d5ce8dfd21ffd3e887ad1c8fbdf2ba1f8968a20808ac0c761bece"),
            felt!("0x5c7fc140fa4fc1a7105a08d9321e128dcaa126877a2446c6e21a6165c338ec5"),
            felt!("0x4"),
            felt!("0x5"),
            felt!("0x30327ce034485166e616b9a45ad0e89307b5adcce1b7d408114958694d3876e"),
            felt!("0x36966db0df331ea61ce0a63e42701132571d60c7dc75b2440a61c8f30db0c61"),
            felt!("0x0"),
            felt!("0x1d6140d8a47e980132a4f31e51d6a82c82e7e1cbef99b2ee92159550c47f5f8"),
            felt!("0x0"),
            felt!("0x0"),
            felt!("0x4"),
            felt!("0xbe8c1b5ddc2edacb375bc8734b8a96d618f8213df8bd531e60fa338c0aa429"),
            felt!("0x3c87be0be4d0ff385fe08d8beb0a1c2861c8133d54dfa73e27b082748b5c2a1"),
            felt!("0x1"),
            felt!("0x6f"),
            felt!("0x0"),
            felt!("0x800000000010000100000000000000a00000"),
            felt!("0x21e12778bee0b852800"),
            felt!("0x7693dcca6bad800"),
            felt!("0x23c045000a011a50080420002"),
            felt!("0xd528b93"),
        ];

        let (messages_to_l1, messages_to_l2) =
            extract_messages_from_program_output(&mut program_output.into_iter());

        assert_eq!(messages_to_l1.len(), 1);
        assert_eq!(messages_to_l2.len(), 0);

        assert_eq!(
            messages_to_l1[0].from_address,
            felt!("0xbe8c1b5ddc2edacb375bc8734b8a96d618f8213df8bd531e60fa338c0aa429")
        );
        assert_eq!(
            messages_to_l1[0].to_address,
            felt!("0x3c87be0be4d0ff385fe08d8beb0a1c2861c8133d54dfa73e27b082748b5c2a1")
        );
        assert_eq!(messages_to_l1[0].payload, vec![felt!("0x6f")]);
    }

    #[test]
    fn test_extract_messages_only_messages_to_l2() {
        let program_output: Vec<Felt> = vec![
            felt!("0x1"),
            felt!("0x0"),
            felt!("0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8"),
            felt!("0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07"),
            felt!("0x0"),
            felt!("0x1"),
            felt!("0x0"),
            felt!("0x10e5341a417427d140af8f5def7d2cc687d84591ff8ec241623c590b5ca8c80"),
            felt!("0x5c7fc140fa4fc1a7105a08d9321e128dcaa126877a2446c6e21a6165c338ec5"),
            felt!("0x5c7fc140fa4fc1a7105a08d9321e128dcaa126877a2446c6e21a6165c338ec5"),
            felt!("0x5"),
            felt!("0x6"),
            felt!("0x36966db0df331ea61ce0a63e42701132571d60c7dc75b2440a61c8f30db0c61"),
            felt!("0x7b0171fbe302ce5aa393dbccb430a8422d57786014faa97767fc8f3ff353e26"),
            felt!("0x0"),
            felt!("0x1d6140d8a47e980132a4f31e51d6a82c82e7e1cbef99b2ee92159550c47f5f8"),
            felt!("0x0"),
            felt!("0x0"),
            felt!("0x0"),
            felt!("0x6"),
            felt!("0x3c87be0be4d0ff385fe08d8beb0a1c2861c8133d54dfa73e27b082748b5c2a1"),
            felt!("0xbe8c1b5ddc2edacb375bc8734b8a96d618f8213df8bd531e60fa338c0aa429"),
            felt!("0x1"),
            felt!("0x5421de947699472df434466845d68528f221a52fce7ad2934c5dae2e1f1cdc"),
            felt!("0x1"),
            felt!("0x378"),
            felt!("0x10000100000000000000000000000000000200000"),
            felt!("0x0"),
            felt!("0x0"),
            felt!("0x2f"),
        ];

        let (messages_to_l1, messages_to_l2) =
            extract_messages_from_program_output(&mut program_output.into_iter());

        assert_eq!(messages_to_l1.len(), 0);
        assert_eq!(messages_to_l2.len(), 1);

        assert_eq!(
            messages_to_l2[0].from_address,
            felt!("0x3c87be0be4d0ff385fe08d8beb0a1c2861c8133d54dfa73e27b082748b5c2a1")
        );
        assert_eq!(
            messages_to_l2[0].to_address,
            felt!("0xbe8c1b5ddc2edacb375bc8734b8a96d618f8213df8bd531e60fa338c0aa429")
        );
        assert_eq!(messages_to_l2[0].nonce, felt!("0x1"));
        assert_eq!(
            messages_to_l2[0].selector,
            felt!("0x5421de947699472df434466845d68528f221a52fce7ad2934c5dae2e1f1cdc")
        );
        assert_eq!(messages_to_l2[0].payload, vec![felt!("0x378")]);
    }
}
