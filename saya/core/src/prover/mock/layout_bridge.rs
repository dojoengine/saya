use anyhow::Result;
use integrity::Felt;
use log::{debug, info};
use starknet_crypto::poseidon_hash_many;
use swiftness::TransformTo;
use swiftness_air::types::SegmentInfo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    prover::{Prover, ProverBuilder, RecursiveProof, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};

/// Prover implementation as a client to the hosted [Mock Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct MockLayoutBridgeProver {
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<RecursiveProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug, Default)]
pub struct MockLayoutBridgeProverBuilder {
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<RecursiveProof>>,
}

impl MockLayoutBridgeProver {
    async fn run(mut self) {
        loop {
            let new_snos_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_snos_proof = self.statement_channel.recv() => new_snos_proof,
            };

            // This should be fine for now as block ingestors wouldn't drop senders. This might
            // change in the future.
            let new_snos_proof = new_snos_proof.unwrap();

            debug!(
                "Receive raw SNOS proof for block #{}",
                new_snos_proof.block_number
            );

            // TODO: error handling
            let parsed_snos_proof: StarkProof = swiftness::parse(&new_snos_proof.proof)
                .unwrap()
                .transform_to();
            let snos_output = calculate_output(&parsed_snos_proof);

            // This proof is mocked but calling `calculate_output` on it correctly yieids the
            // expected output.
            //
            // This spaghetti is needed because `StarkProof` does not implement `Default`.
            let mock_proof = StarkProof {
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
                            stop_ptr: Felt::from(5),
                        },
                    ],
                    padding_addr: Default::default(),
                    padding_value: Default::default(),
                    main_page: swiftness_air::types::Page(
                        [
                            Felt::ZERO,
                            Felt::ZERO,
                            Felt::ZERO,
                            Felt::ZERO,
                            poseidon_hash_many(&snos_output),
                        ]
                        .into_iter()
                        .map(|value| swiftness_air::types::AddrValue {
                            address: Default::default(),
                            value,
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
            };

            let new_proof = RecursiveProof {
                block_number: new_snos_proof.block_number,
                snos_output,
                layout_bridge_proof: mock_proof,
            };

            info!(
                "Mock proof generated for block #{}",
                new_snos_proof.block_number
            );

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.proof_channel.send(new_proof) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl MockLayoutBridgeProverBuilder {
    pub fn new() -> Self {
        Self {
            statement_channel: None,
            proof_channel: None,
        }
    }
}

impl ProverBuilder for MockLayoutBridgeProverBuilder {
    type Prover = MockLayoutBridgeProver;

    fn build(self) -> Result<Self::Prover> {
        Ok(MockLayoutBridgeProver {
            statement_channel: self
                .statement_channel
                .ok_or_else(|| anyhow::anyhow!("`statement_channel` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn statement_channel(mut self, statement_channel: Receiver<SnosProof<String>>) -> Self {
        self.statement_channel = Some(statement_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<RecursiveProof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }
}

impl Prover for MockLayoutBridgeProver {
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl Daemon for MockLayoutBridgeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
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
