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
    block_ingestor::BlockInfo,
    prover::{
        atlantic::{client::AtlanticClient, snos::compress_pie},
        error::ProverError,
        LayoutBridgeTraceGenerator, Prover, ProverBuilder, RecursiveProof, SnosProof,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{PersistantStorage, Step},
    utils::calculate_output,
};

use super::client::AtlanticQueryStatus;

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticLayoutBridgeProver<T, DB> {
    client: AtlanticClient,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    trace_generator: T,
    db: DB,
    workers_count: usize,
}

#[derive(Debug)]
pub struct AtlanticLayoutBridgeProverBuilder<T, DB> {
    api_key: String,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<BlockInfo>>,
    trace_generator: T,
    db: DB,
    workers_count: usize,
}

impl<T, DB> AtlanticLayoutBridgeProver<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<SnosProof<String>>>>,
        task_tx: Sender<BlockInfo>,
        client: AtlanticClient,
        layout_bridge: Cow<'static, [u8]>,
        trace_generator: T,
        finish_handle: FinishHandle,
        db: DB,
    ) where
        T: LayoutBridgeTraceGenerator<DB> + Send + Sync + 'static,
        DB: PersistantStorage + Send + Sync + 'static,
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

            let block_number_u32 = new_snos_proof.block_number.try_into().unwrap();

            match db
                .get_proof(block_number_u32, crate::storage::Step::Bridge)
                .await
            {
                Ok(proof) => {
                    info!(
                        "Proof already generated for block #{}",
                        new_snos_proof.block_number
                    );
                    let verifier_proof = String::from_utf8(proof).unwrap();
                    let _: StarkProof = swiftness::parse(verifier_proof).unwrap().transform_to(); //Sanity check if the proof is valid

                    info!("Proof generated for block #{}", new_snos_proof.block_number);

                    let new_proof = BlockInfo {
                        number: new_snos_proof.block_number,
                        status: crate::storage::BlockStatus::SnosProofGenerated,
                    };
                    task_tx.send(new_proof).await.unwrap();
                    continue;
                }
                Err(_) => {
                    info!(
                        "Proof not generated for block #{}",
                        new_snos_proof.block_number
                    );
                }
            }
            match db
                .get_query_id(block_number_u32, crate::storage::Query::BridgeProof)
                .await
            {
                Ok(atlantic_query_id) => {
                    info!(
                        "Proof generation already submitted for block #{}",
                        new_snos_proof.block_number
                    );
                    match Self::wait_for_proof(
                        client.clone(),
                        atlantic_query_id.clone(),
                        finish_handle.clone(),
                    )
                    .await
                    {
                        Err(ProverError::Shutdown) => {
                            break;
                        }
                        Err(ProverError::BlockFail(e)) => {
                            log::error!("{}", e,);
                            db.add_failed_block(block_number_u32, e).await.unwrap();
                            continue;
                        }
                        Err(_) => {}
                        Ok(_) => {}
                    }
                    debug!(
                        "Atlantic layout bridge proof generation finished for query: {}",
                        atlantic_query_id
                    );
                    let new_proof = BlockInfo {
                        number: new_snos_proof.block_number,
                        status: crate::storage::BlockStatus::SnosProofGenerated,
                    };
                    task_tx.send(new_proof).await.unwrap();
                    continue;
                }
                Err(_) => {
                    info!(
                        "Proof generation not submitted for block #{}",
                        new_snos_proof.block_number
                    );
                }
            }
            let compressed_pie = match db.get_pie(block_number_u32, Step::Bridge).await {
                Ok(pie) => pie,
                Err(_) => {
                    // Hacky way to wrap proof due to the lack of serialization support for the parsed type4
                    // TODO: patch `swiftness` and fix this
                    let input = format!("{{\n\t\"proof\": {}\n}}", new_snos_proof.proof);
                    let label = format!("bench2_layout-trace-{}", new_snos_proof.block_number);

                    // This call fails a lot on atlantic.
                    let layout_bridge_pie = {
                        let mut attempts = 0;
                        const MAX_ATTEMPTS: u32 = 3;

                        loop {
                            match trace_generator
                                .generate_trace(
                                    layout_bridge.clone().to_vec(),
                                    block_number_u32,
                                    &label,
                                    input.clone().into_bytes(),
                                    db.clone(),
                                )
                                .await
                            {
                                Ok(pie) => break pie,
                                Err(e) => {
                                    attempts += 1;
                                    if attempts >= MAX_ATTEMPTS {
                                        panic!(
                                            "Failed to generate trace after {} attempts: {}",
                                            MAX_ATTEMPTS, e
                                        );
                                    }
                                    debug!(
                                        "Trace generation attempt {} failed: {}. Retrying...",
                                        attempts, e
                                    );
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                }
                            }
                        }
                    };

                    let compressed_pie = compress_pie(layout_bridge_pie).await.unwrap();

                    db.add_pie(block_number_u32, compressed_pie.clone(), Step::Bridge)
                        .await
                        .unwrap();

                    compressed_pie
                }
            };

            let atlantic_query_id = crate::utils::retry_with_backoff(
                || {
                    client.submit_proof_generation(
                        compressed_pie.clone(),
                        "recursive_with_poseidon".to_string(),
                        format!("bench2_layout-{}", new_snos_proof.block_number),
                    )
                },
                "submit_proof_generation",
                3,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            crate::utils::retry_with_backoff(
                || {
                    db.add_query_id(
                        new_snos_proof.block_number.try_into().unwrap(),
                        atlantic_query_id.clone(),
                        crate::storage::Query::BridgeProof,
                    )
                },
                "add_query_id",
                3,
                Duration::from_secs(2),
            )
            .await
            .unwrap();

            info!(
                "Atlantic layout bridge proof generation submitted for block #{}: {}",
                new_snos_proof.block_number, atlantic_query_id
            );

            // Wait for bridge layout proof to be done
            match Self::wait_for_proof(
                client.clone(),
                atlantic_query_id.clone(),
                finish_handle.clone(),
            )
            .await
            {
                Err(ProverError::Shutdown) => {
                    break;
                }
                Err(ProverError::BlockFail(e)) => {
                    log::error!("{}", e,);
                    db.add_failed_block(block_number_u32, e).await.unwrap();
                    continue;
                }
                Err(_) => {}
                Ok(_) => {}
            }

            debug!(
                "Atlantic layout bridge proof generation finished for query: {}",
                atlantic_query_id
            );
            let _ = Self::get_and_save_proof(
                client.clone(),
                db.clone(),
                atlantic_query_id,
                block_number_u32,
                parsed_snos_proof,
            )
            .await;
            let new_proof = BlockInfo {
                number: new_snos_proof.block_number,
                status: crate::storage::BlockStatus::SnosProofGenerated,
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
        for _ in 0..self.workers_count {
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
                self.db.clone(),
            )));
        }
        futures_util::future::join_all(workers).await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
    async fn wait_for_proof(
        client: AtlanticClient,
        atlantic_query_id: String,
        finish_handle: FinishHandle,
    ) -> Result<(), ProverError> {
        loop {
            // TODO: sleep with graceful shutdown
            tokio::time::sleep(PROOF_STATUS_POLL_INTERVAL).await;
            if finish_handle.is_shutdown_requested() {
                return Err(ProverError::Shutdown);
            }
            if let Ok(jobs) = client.clone().get_atlantic_query(&atlantic_query_id).await {
                match jobs.atlantic_query.status {
                    AtlanticQueryStatus::Done => break,
                    AtlanticQueryStatus::Failed => {
                        return Err(ProverError::BlockFail(format!(
                            "Proof generation failed for query: {}",
                            atlantic_query_id
                        )));
                    }
                    _ => continue,
                }
            }
        }
        Ok(())
    }
    async fn get_and_save_proof(
        client: AtlanticClient,
        db: DB,
        atlantic_query_id: String,
        block_number: u32,
        parsed_snos_proof: StarkProof,
    ) -> RecursiveProof {
        let verifier_proof = client.get_proof(&atlantic_query_id).await.unwrap();

        crate::utils::retry_with_backoff(
            || {
                db.add_proof(
                    block_number,
                    verifier_proof.as_bytes().to_vec(),
                    crate::storage::Step::Bridge,
                )
            },
            "add_proof",
            3,
            Duration::from_secs(2),
        )
        .await
        .unwrap();

        // TODO: error handling
        let verifier_proof: StarkProof = swiftness::parse(verifier_proof).unwrap().transform_to();

        info!("Proof generated for block #{}", block_number);

        RecursiveProof {
            block_number: block_number as u64,
            snos_output: calculate_output(&parsed_snos_proof),
            layout_bridge_proof: verifier_proof,
        }
    }
}

impl<T, DB> AtlanticLayoutBridgeProverBuilder<T, DB> {
    pub fn new<P>(
        api_key: String,
        layout_bridge: P,
        trace_generator: T,
        db: DB,
        workers_count: usize,
    ) -> Self
    where
        P: Into<Cow<'static, [u8]>>,
        T: LayoutBridgeTraceGenerator<DB> + Send + Sync + 'static,
        DB: PersistantStorage + Send + Sync + Clone + 'static,
    {
        Self {
            api_key,
            layout_bridge: layout_bridge.into(),
            statement_channel: None,
            proof_channel: None,
            trace_generator,
            db,
            workers_count,
        }
    }
}

impl<T, DB> ProverBuilder for AtlanticLayoutBridgeProverBuilder<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Prover = AtlanticLayoutBridgeProver<T, DB>;

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
            db: self.db,
            workers_count: self.workers_count,
        })
    }

    fn statement_channel(mut self, statement_channel: Receiver<SnosProof<String>>) -> Self {
        self.statement_channel = Some(statement_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<BlockInfo>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }
}

impl<T, DB> Prover for AtlanticLayoutBridgeProver<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Clone + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Statement = SnosProof<String>;
    type Proof = BlockInfo;
}

impl<T, DB> Daemon for AtlanticLayoutBridgeProver<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Clone + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
