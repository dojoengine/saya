use std::{borrow::Cow, sync::Arc, time::Duration};

use crate::{
    block_ingestor::BlockInfo,
    prover::{
        atlantic::{
            calculate_job_size,
            client::{AtlanticClient, Layout},
            snos::compress_pie,
        },
        error::ProverError,
        Prover, ProverBuilder, SnosProof,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{PersistantStorage, Step},
};
use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use log::{debug, info, trace};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex,
};

use super::client::AtlanticQueryStatus;

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticLayoutBridgeProver<DB> {
    client: AtlanticClient,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
}

#[derive(Debug)]
pub struct AtlanticLayoutBridgeProverBuilder<DB> {
    api_key: String,
    layout_bridge: Cow<'static, [u8]>,
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
}

impl<DB> AtlanticLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<SnosProof<String>>>>,
        task_tx: Sender<BlockInfo>,
        client: AtlanticClient,
        layout_bridge: Cow<'static, [u8]>,
        finish_handle: FinishHandle,
        db: DB,
    ) where
        DB: PersistantStorage + Send + Sync + 'static,
    {
        loop {
            let new_snos_proof = if let Some(new_block) = task_rx.lock().await.recv().await {
                new_block
            } else {
                break;
            };

            let block_number_u32 = new_snos_proof.block_number.try_into().unwrap();

            match db
                .get_proof(block_number_u32, crate::storage::Step::Bridge)
                .await
            {
                Ok(proof) => {
                    let verifier_proof = String::from_utf8(proof).unwrap();
                    let proof = swiftness::parse(verifier_proof); //Sanity check if the proof is valid
                    if proof.is_ok() {
                        trace!(
                            block_number = new_snos_proof.block_number;
                            "Proof already generated for block"
                        );
                        let block_info = BlockInfo {
                            number: new_snos_proof.block_number,
                            status: crate::storage::BlockStatus::SnosProofGenerated,
                        };

                        task_tx.send(block_info).await.unwrap();
                        continue;
                    }
                }
                Err(_) => {
                    trace!(
                        block_number = new_snos_proof.block_number;
                        "Proof not found in db for block",
                    );
                }
            }

            match db
                .get_query_id(block_number_u32, crate::storage::Query::BridgeProof)
                .await
            {
                Ok(atlantic_query_id) => {
                    info!(block_number = new_snos_proof.block_number; "Proof generation already submitted for block");
                    match Self::wait_for_job(
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
                            log::error!(error:% = e, atlantic_query_id:% = atlantic_query_id; "Proof generation failed");
                            db.add_failed_block(block_number_u32, e).await.unwrap();
                            continue;
                        }
                        Err(_) => {}
                        Ok(_) => {}
                    }

                    debug!(
                        atlantic_query_id:? = atlantic_query_id;
                        "Atlantic layout bridge proof generation finished"
                    );

                    Self::get_and_save_proof(
                        client.clone(),
                        db.clone(),
                        atlantic_query_id,
                        block_number_u32,
                    )
                    .await;

                    let new_proof = BlockInfo {
                        number: new_snos_proof.block_number,
                        status: crate::storage::BlockStatus::SnosProofGenerated,
                    };

                    task_tx.send(new_proof).await.unwrap();
                    continue;
                }
                Err(_) => {
                    trace!(
                        block_number = new_snos_proof.block_number;
                        "Proof generation not submitted for block"
                    );
                }
            }

            let compressed_pie = match db.get_pie(block_number_u32, Step::Bridge).await {
                Ok(pie) => pie,
                Err(_) => {
                    // Hacky way to wrap proof due to the lack of serialization support for the parsed type4
                    // TODO: patch `swiftness` and fix this
                    let input = format!("{{\n\t\"proof\": {}\n}}", new_snos_proof.proof);
                    let label = format!("layout-trace-{}", new_snos_proof.block_number);

                    let atlantic_query_id = match db
                        .get_query_id(block_number_u32, crate::storage::Query::BridgeTrace)
                        .await
                    {
                        Ok(query_id) => query_id,
                        Err(_) => {
                            let atlantic_query_id = crate::utils::retry_with_backoff(
                                || {
                                    client.submit_trace_generation(
                                        &label,
                                        layout_bridge.clone().to_vec(),
                                        input.clone().into_bytes(),
                                    )
                                },
                                "trace_gen",
                                3,
                                Duration::from_secs(5),
                            )
                            .await
                            .unwrap();

                            db.add_query_id(
                                block_number_u32,
                                atlantic_query_id.clone(),
                                crate::storage::Query::BridgeTrace,
                            )
                            .await
                            .unwrap();

                            atlantic_query_id
                        }
                    };

                    info!(
                        block_number = new_snos_proof.block_number,
                        atlantic_query_id:? = atlantic_query_id;
                        "Atlantic trace generation submitted",
                    );

                    match Self::wait_for_job(
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
                    };

                    let pie_bytes = client.get_trace(&atlantic_query_id).await.unwrap();
                    let layout_bridge_pie = CairoPie::from_bytes(&pie_bytes).unwrap();

                    let compressed_pie = compress_pie(layout_bridge_pie).await.unwrap();

                    db.add_pie(block_number_u32, compressed_pie.clone(), Step::Bridge)
                        .await
                        .unwrap();

                    compressed_pie
                }
            };
            let atlantic_job_size =
                calculate_job_size(CairoPie::from_bytes(&compressed_pie).unwrap());
            let atlantic_query_id = crate::utils::retry_with_backoff(
                || {
                    client.submit_proof_generation(
                        compressed_pie.clone(),
                        Layout::recursive_with_poseidon,
                        format!("layout-{}", new_snos_proof.block_number),
                        atlantic_job_size,
                    )
                },
                "submit_proof_generation",
                3,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            db.add_query_id(
                new_snos_proof.block_number.try_into().unwrap(),
                atlantic_query_id.clone(),
                crate::storage::Query::BridgeProof,
            )
            .await
            .unwrap();

            info!(
                block_number = new_snos_proof.block_number,
                atlantic_query_id:? = atlantic_query_id;
                "Atlantic layout bridge proof generation submitted",
            );

            // Wait for bridge layout proof to be done
            match Self::wait_for_job(
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
                    log::error!(error:% = e, atlantic_query_id:% = atlantic_query_id; "Proof generation failed");
                    db.add_failed_block(block_number_u32, e).await.unwrap();
                    continue;
                }
                Err(_) => {}
                Ok(_) => {}
            }

            Self::get_and_save_proof(
                client.clone(),
                db.clone(),
                atlantic_query_id.clone(),
                block_number_u32,
            )
            .await;

            debug!(
                block_number = new_snos_proof.block_number,
                atlantic_query_id:? = atlantic_query_id;
                "Atlantic layout bridge proof generation finished",
            );

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
            let finish_handle = self.finish_handle.clone();
            workers.push(tokio::spawn(Self::worker(
                worker_task_rx,
                task_tx,
                client,
                layout_bridge,
                finish_handle,
                self.db.clone(),
            )));
        }
        futures_util::future::join_all(workers).await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn wait_for_job(
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
    ) {
        let verifier_proof = client.get_proof(&atlantic_query_id).await.unwrap();

        db.add_proof(
            block_number,
            verifier_proof.as_bytes().to_vec(),
            crate::storage::Step::Bridge,
        )
        .await
        .unwrap();
    }
}
impl<DB> AtlanticLayoutBridgeProverBuilder<DB> {
    pub fn new<P>(api_key: String, layout_bridge: P, db: DB, workers_count: usize) -> Self
    where
        P: Into<Cow<'static, [u8]>>,
        DB: PersistantStorage + Send + Sync + Clone + 'static,
    {
        Self {
            api_key,
            layout_bridge: layout_bridge.into(),
            statement_channel: None,
            proof_channel: None,
            db,
            workers_count,
        }
    }
}

impl<DB> ProverBuilder for AtlanticLayoutBridgeProverBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Prover = AtlanticLayoutBridgeProver<DB>;

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

impl<DB> Prover for AtlanticLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Statement = SnosProof<String>;
    type BlockInfo = BlockInfo;
}

impl<DB> Daemon for AtlanticLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
