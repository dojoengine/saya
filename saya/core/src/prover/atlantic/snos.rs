use std::{io::Write, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use log::{debug, info, trace};
use tokio::sync::mpsc::{Receiver, Sender};
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
};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticSnosProver<P> {
    client: AtlanticClient,
    statement_channel: Receiver<NewBlock>,
    proof_channel: Sender<SnosProof<P>>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct AtlanticSnosProverBuilder<P> {
    api_key: String,
    statement_channel: Option<Receiver<NewBlock>>,
    proof_channel: Option<Sender<SnosProof<P>>>,
}

impl<P> AtlanticSnosProver<P>
where
    P: AtlanticProof,
{
    async fn run(mut self) {
        // TODO: add persistence for in-flight proof requests to be able to resume progress

        loop {
            let new_block = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_block = self.statement_channel.recv() => new_block,
            };

            // This should be fine for now as block ingestors wouldn't drop senders. This might
            // change in the future.
            let new_block = new_block.unwrap();

            trace!("Compressing PIE for block #{}", new_block.number);

            // TODO: error handling
            let compressed_pie = compress_pie(&new_block.pie).unwrap();
            debug!(
                "Compressed PIE size for block #{}: {} bytes",
                new_block.number,
                compressed_pie.len()
            );

            // TODO: error handling
            let atlantic_query_id = self
                .client
                .submit_proof_generation(compressed_pie)
                .await
                .unwrap();

            info!(
                "Atlantic proof generation submitted for block #{}: {}",
                new_block.number, atlantic_query_id
            );

            // Wait for PIE proof to be done
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
            let raw_proof = self.client.get_proof(&atlantic_query_id).await.unwrap();

            // TODO: error handling
            let parsed_proof: P = P::parse(raw_proof).unwrap();

            info!("Proof generated for block #{}", new_block.number);

            let new_proof = SnosProof {
                block_number: new_block.number,
                proof: parsed_proof,
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

impl<P> AtlanticSnosProverBuilder<P> {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            statement_channel: None,
            proof_channel: None,
        }
    }
}

impl<P> ProverBuilder for AtlanticSnosProverBuilder<P>
where
    P: AtlanticProof + Send + 'static,
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
    P: AtlanticProof + Send + 'static,
{
    type Statement = NewBlock;
    type Proof = SnosProof<P>;
}

impl<P> Daemon for AtlanticSnosProver<P>
where
    P: AtlanticProof + Send + 'static,
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
fn compress_pie(pie: &CairoPie) -> std::result::Result<Vec<u8>, std::io::Error> {
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
}
