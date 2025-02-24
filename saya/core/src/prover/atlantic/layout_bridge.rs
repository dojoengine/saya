use std::{borrow::Cow, sync::Arc, time::Duration};

use anyhow::Result;
use log::{debug, info};
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex,
};

use crate::{
    prover::{
        atlantic::{
            client::{AtlanticClient, AtlanticJobStatus},
            snos::compress_pie,
            PROOF_GENERATION_JOB_NAME,
        },
        LayoutBridgeTraceGenerator, Prover, ProverBuilder, RecursiveProof, SnosProof,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const WORKER_COUNT: usize = 10;
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticLayoutBridgeProver<T> {
    client: AtlanticClient,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<RecursiveProof>,
    finish_handle: FinishHandle,
    trace_generator: T,
}

#[derive(Debug)]
pub struct AtlanticLayoutBridgeProverBuilder<T> {
    api_key: String,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<RecursiveProof>>,
    trace_generator: T,
}

impl<T> AtlanticLayoutBridgeProver<T>
where
    T: LayoutBridgeTraceGenerator + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<SnosProof<String>>>>,
        task_tx: Sender<RecursiveProof>,
        client: AtlanticClient,
        layout_bridge: Cow<'static, [u8]>,
        trace_generator: T,
        finish_handle: FinishHandle,
    ) where
        T: LayoutBridgeTraceGenerator + Send + Sync + 'static,
    {
        loop {
            let new_snos_proof = if let Some(new_block) = task_rx.lock().await.recv().await {
                new_block
            } else {
                break;
            };
            debug!(
                "Receive raw SNOS proof for block #{}",
                new_snos_proof.block_number
            );
            let parsed_snos_proof: StarkProof = swiftness::parse(&new_snos_proof.proof)
                .unwrap()
                .transform_to();
            // Hacky way to wrap proof due to the lack of serialization support for the parsed type4
            // TODO: patch `swiftness` and fix this
            let input = format!("{{\n\t\"proof\": {}\n}}", new_snos_proof.proof);
            //trace gen Trait executed here.
            let layout_bridge_pie = trace_generator
                .generate_trace(layout_bridge.clone().to_vec(), input.into_bytes(), Some(format!("trace_layout_{}", new_snos_proof.block_number)))
                .await
                .unwrap();

            let compressed_pie = compress_pie(layout_bridge_pie).await.unwrap();
            let atlantic_query_id = client
                .submit_proof_generation(
                    compressed_pie,
                    "recursive_with_poseidon".to_string(),
                    format!("layout_{}", new_snos_proof.block_number),
                )
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
                if let Ok(jobs) = client.get_query_jobs(&atlantic_query_id).await {
                    if let Some(proof_generation_job) = jobs
                        .iter()
                        .find(|job| job.job_name == PROOF_GENERATION_JOB_NAME)
                    {
                        match proof_generation_job.status {
                            AtlanticJobStatus::Completed => break,
                            AtlanticJobStatus::Failed => {
                                // TODO: error handling
                                panic!("Atlantic proof generation {} failed", atlantic_query_id);
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
            let verifier_proof = client.get_proof(&atlantic_query_id).await.unwrap();

            // TODO: error handling
            let verifier_proof: StarkProof =
                swiftness::parse(verifier_proof).unwrap().transform_to();

            info!("Proof generated for block #{}", new_snos_proof.block_number);

            let new_proof = RecursiveProof {
                block_number: new_snos_proof.block_number,
                snos_output: calculate_output(&parsed_snos_proof),
                layout_bridge_proof: verifier_proof,
            };

            tokio::select! {
                _ = finish_handle.shutdown_requested() => break,
                _ = task_tx.send(new_proof) => {},
            }
        }
    }
    async fn run(self) {
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(self.statement_channel));
        for _ in 0..WORKER_COUNT {
            let worker_task_rx = task_rx.clone();
            let task_tx = self.proof_channel.clone();
            let client = self.client.clone();
            let layout_bridge = self.layout_bridge.clone();
            let trace_generator = self.trace_generator.clone();
            let finish_handle = self.finish_handle.clone();
            workers.push(tokio::spawn(Self::worker(
                worker_task_rx,
                task_tx,
                client,
                layout_bridge,
                trace_generator,
                finish_handle,
            )));
        }
        futures_util::future::join_all(workers).await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<T> AtlanticLayoutBridgeProverBuilder<T> {
    pub fn new<P>(api_key: String, layout_bridge: P, trace_generator: T) -> Self
    where
        P: Into<Cow<'static, [u8]>>,
        T: LayoutBridgeTraceGenerator + Send + Sync + 'static,
    {
        Self {
            api_key,
            layout_bridge: layout_bridge.into(),
            statement_channel: None,
            proof_channel: None,
            trace_generator,
        }
    }
}

impl<T> ProverBuilder for AtlanticLayoutBridgeProverBuilder<T>
where
    T: LayoutBridgeTraceGenerator + Send + Sync + Clone + 'static,
{
    type Prover = AtlanticLayoutBridgeProver<T>;

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
            trace_generator: self.trace_generator,
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

impl<T> Prover for AtlanticLayoutBridgeProver<T>
where
    T: LayoutBridgeTraceGenerator + Send + Clone + Sync + 'static,
{
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl<T> Daemon for AtlanticLayoutBridgeProver<T>
where
    T: LayoutBridgeTraceGenerator + Send + Clone + Sync + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
