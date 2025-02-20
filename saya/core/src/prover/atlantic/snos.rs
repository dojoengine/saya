use std::{io::Write, sync::Arc, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use log::{debug, info, trace};
use starknet::core::types::Felt;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    task,
};
use zip::{write::FileOptions, ZipWriter};

use crate::{
    block_ingestor::NewBlock,
    prover::{
        atlantic::{
            client::{AtlanticClient, AtlanticJobStatus},
            AtlanticProof, PROOF_GENERATION_JOB_NAME,
        },
        Prover, ProverBuilder, SnosProof,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::{compute_program_hash_from_pie, extract_pie_output, stark_proof_mock},
};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const WORKER_COUNT: usize = 10;
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticSnosProver<P> {
    client: AtlanticClient,
    statement_channel: Receiver<NewBlock>,
    proof_channel: Sender<SnosProof<P>>,
    finish_handle: FinishHandle,
    /// Whether to extract the output and compute the program hash from the PIE or use the one from the SHARP bootloader returned by the prover service.
    mock_snos_from_pie: bool,
}

#[derive(Debug)]
pub struct AtlanticSnosProverBuilder<P> {
    api_key: String,
    statement_channel: Option<Receiver<NewBlock>>,
    proof_channel: Option<Sender<SnosProof<P>>>,
    mock_snos_from_pie: bool,
}

impl<P> AtlanticSnosProver<P>
where
    P: AtlanticProof + Send + Sync + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<NewBlock>>>,
        task_tx: Sender<SnosProof<P>>,
        client: AtlanticClient,
        finish_handle: FinishHandle,
        mock_snos_from_pie: bool,
    ) where
        P: AtlanticProof,
    {
        loop {
            let new_block = if let Some(new_block) = task_rx.lock().await.recv().await {
                new_block
            } else {
                break;
            };

            // TODO: move this to a separate MockSnosProver.
            if mock_snos_from_pie {
                let output = bootloader_snos_output(&new_block.pie);
                let mock_proof = stark_proof_mock(&output);

                info!(
                    "Mock proof generated from PIE for block #{}",
                    new_block.number
                );

                let new_proof = SnosProof {
                    block_number: new_block.number,
                    proof: P::parse(serde_json::to_string(&mock_proof).unwrap()).unwrap(),
                };

                let _ = task_tx.send(new_proof).await;
                continue;
            }

            trace!("Compressing PIE for block #{}", new_block.number);

            // TODO: error handling
            let compressed_pie = compress_pie(new_block.pie).await.unwrap();

            debug!(
                "Compressed PIE size for block #{}: {} bytes",
                new_block.number,
                compressed_pie.len()
            );

            let atlantic_query_id = client
                .submit_proof_generation(
                    compressed_pie,
                    "dynamic".to_string(),
                    format!("block-{}", new_block.number),
                )
                .await
                .unwrap();

            info!(
                "Atlantic proof generation submitted for block #{}: {}",
                new_block.number, atlantic_query_id
            );
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
                "Atlantic PIE proof generation finished for query: {}",
                atlantic_query_id
            );
            // TODO: error handling
            let raw_proof = client.get_proof(&atlantic_query_id).await.unwrap();

            // TODO: error handling
            let parsed_proof: P = P::parse(raw_proof).unwrap();

            info!("Proof generated for block #{}", new_block.number);

            let new_proof = SnosProof {
                block_number: new_block.number,
                proof: parsed_proof,
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
            let worker_task_tx = self.proof_channel.clone();
            workers.push(task::spawn(Self::worker(
                task_rx.clone(),
                worker_task_tx,
                self.client.clone(),
                self.finish_handle.clone(),
                self.mock_snos_from_pie,
            )));
        }
        futures_util::future::join_all(workers).await;
        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<P> AtlanticSnosProverBuilder<P> {
    pub fn new(api_key: String, mock_snos_from_pie: bool) -> Self {
        Self {
            api_key,
            statement_channel: None,
            proof_channel: None,
            mock_snos_from_pie,
        }
    }
}

impl<P> ProverBuilder for AtlanticSnosProverBuilder<P>
where
    P: AtlanticProof + Send + Sync + 'static,
{
    type Prover = AtlanticSnosProver<P>;

    fn build(self) -> Result<Self::Prover> {
        Ok(AtlanticSnosProver {
            client: AtlanticClient::new(self.api_key),
            statement_channel: self
                .statement_channel
                .ok_or_else(|| anyhow::anyhow!("`statement_channel` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            mock_snos_from_pie: self.mock_snos_from_pie,
        })
    }

    fn statement_channel(mut self, statement_channel: Receiver<NewBlock>) -> Self {
        self.statement_channel = Some(statement_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<SnosProof<P>>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }
}

impl<P> Prover for AtlanticSnosProver<P>
where
    P: AtlanticProof + Send + Sync + 'static,
{
    type Statement = NewBlock;
    type Proof = SnosProof<P>;
}

impl<P> Daemon for AtlanticSnosProver<P>
where
    P: AtlanticProof + Send + Sync + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

/// Converts a `CairoPie` instance into a Zip archive accepted by the Atlantic prover service.
///
/// Unfortunately `cairo-vm` does not offer a generic API for converting PIE to Zip bytes that
/// doesn't involve using the file system. This is mostly copied from `CairoPie::write_zip_file`.
pub async fn compress_pie(pie: CairoPie) -> std::result::Result<Vec<u8>, std::io::Error> {
    task::spawn_blocking(move || {
        let mut bytes = std::io::Cursor::new(Vec::<u8>::new());
        let mut zip_writer = ZipWriter::new(&mut bytes);
        let options =
            FileOptions::<'_, ()>::default().compression_method(zip::CompressionMethod::Deflated);

        zip_writer.start_file("version.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.version)?;
        zip_writer.start_file("metadata.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.metadata)?;
        zip_writer.start_file("memory.bin", options)?;
        zip_writer.write_all(&pie.memory.to_bytes())?;
        zip_writer.start_file("additional_data.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.additional_data)?;
        zip_writer.start_file("execution_resources.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.execution_resources)?;
        zip_writer.finish()?;

        Ok(bytes.into_inner())
    })
    .await?
}

/// Mocks a bootloaded execution of SNOS.
fn bootloader_snos_output(pie: &CairoPie) -> Vec<Felt> {
    let snos_program_hash =
        compute_program_hash_from_pie(pie).expect("Failed to compute program hash from PIE");
    debug!("SNOS program hash from PIE: {:x}", snos_program_hash);

    let snos_output = extract_pie_output(pie);

    let mut bootloader_output = vec![
        // Bootloader config (not checked by piltover, set to 0)
        Felt::ZERO,
        // bootloader output len (not checked by piltover, set to 0)
        Felt::ZERO,
        snos_program_hash,
    ];

    bootloader_output.extend(snos_output);
    bootloader_output
}
