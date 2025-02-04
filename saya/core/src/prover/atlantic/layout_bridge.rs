use std::{borrow::Cow, time::Duration};

use anyhow::Result;
use log::{debug, info};
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    prover::{
        atlantic::{
            client::{AtlanticClient, AtlanticJobStatus},
            PROOF_GENERATION_JOB_NAME,
        },
        mock::StarkProofMockBuilder,
        Prover, ProverBuilder, RecursiveProof, SnosProof,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticLayoutBridgeProver {
    client: AtlanticClient,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<RecursiveProof>,
    finish_handle: FinishHandle,
    is_mocked: bool,
}

#[derive(Debug)]
pub struct AtlanticLayoutBridgeProverBuilder {
    api_key: String,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<RecursiveProof>>,
    is_mocked: bool,
}

impl AtlanticLayoutBridgeProver {
    async fn run(mut self) {
        // TODO: add persistence for in-flight proof requests to be able to resume progress

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

            let new_proof = if self.is_mocked {
                info!("Proof mocked for block #{}", new_snos_proof.block_number);

                let snos_output = calculate_output(&parsed_snos_proof);
                let layout_bridge_proof = StarkProof::mock_from_output(&snos_output);

                RecursiveProof {
                    block_number: new_snos_proof.block_number,
                    snos_output,
                    layout_bridge_proof,
                }
            } else {
                // Hacky way to wrap proof due to the lack of serialization support for the parsed type
                // TODO: patch `swiftness` and fix this
                let input = format!("{{\n\t\"proof\": {}\n}}", new_snos_proof.proof);

                // TODO: error handling
                let atlantic_query_id = self
                    .client
                    .submit_l2_atlantic_query(self.layout_bridge.clone(), input.into_bytes())
                    .await
                    .unwrap();

                info!(
                    "Atlantic layout bridge proof generation submitted for block #{}: {}",
                    new_snos_proof.block_number, atlantic_query_id
                );

                // Wait for bridge layout proof to be done
                loop {
                    // TODO: sleep with graceful shutdown
                    tokio::time::sleep(PROOF_STATUS_POLL_INTERVAL).await;

                    // TODO: error handling
                    if let Ok(jobs) = self.client.get_query_jobs(&atlantic_query_id).await {
                        if let Some(proof_generation_job) = jobs
                            .iter()
                            .find(|job| job.job_name == PROOF_GENERATION_JOB_NAME)
                        {
                            match proof_generation_job.status {
                                AtlanticJobStatus::Completed => break,
                                AtlanticJobStatus::Failed => {
                                    // TODO: error handling
                                    panic!(
                                        "Atlantic proof generation {} failed",
                                        atlantic_query_id
                                    );
                                }
                                AtlanticJobStatus::InProgress => {}
                            }
                        }
                    }
                }

                debug!(
                    "Atlantic layout bridge proof generation finished for query: {}",
                    atlantic_query_id
                );

                // TODO: error handling
                let verifier_proof = self.client.get_proof(&atlantic_query_id).await.unwrap();

                // TODO: error handling
                let verifier_proof: StarkProof =
                    swiftness::parse(verifier_proof).unwrap().transform_to();

                info!("Proof generated for block #{}", new_snos_proof.block_number);

                RecursiveProof {
                    block_number: new_snos_proof.block_number,
                    snos_output: calculate_output(&parsed_snos_proof),
                    layout_bridge_proof: verifier_proof,
                }
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.proof_channel.send(new_proof) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl AtlanticLayoutBridgeProverBuilder {
    pub fn new<P>(api_key: String, layout_bridge: P, is_mocked: bool) -> Self
    where
        P: Into<Cow<'static, [u8]>>,
    {
        Self {
            api_key,
            layout_bridge: layout_bridge.into(),
            statement_channel: None,
            proof_channel: None,
            is_mocked,
        }
    }
}

impl ProverBuilder for AtlanticLayoutBridgeProverBuilder {
    type Prover = AtlanticLayoutBridgeProver;

    fn build(self) -> Result<Self::Prover> {
        Ok(AtlanticLayoutBridgeProver {
            client: AtlanticClient::new(self.api_key),
            layout_bridge: self.layout_bridge,
            statement_channel: self
                .statement_channel
                .ok_or_else(|| anyhow::anyhow!("`statement_channel` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            is_mocked: self.is_mocked,
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

impl Prover for AtlanticLayoutBridgeProver {
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl Daemon for AtlanticLayoutBridgeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
