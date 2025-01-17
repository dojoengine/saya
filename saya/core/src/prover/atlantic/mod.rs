use std::{io::Write, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use log::{debug, info, trace};
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};
use zip::{write::FileOptions, ZipWriter};

use crate::{
    block_ingestor::NewBlock,
    prover::{Proof, Prover, ProverBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

mod client;
use client::{AtlanticClient, AtlanticQueryStatus};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticProver {
    client: AtlanticClient,
    block_channel: Receiver<NewBlock>,
    proof_channel: Sender<Proof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct AtlanticProverBuilder {
    api_key: String,
    block_channel: Option<Receiver<NewBlock>>,
    proof_channel: Option<Sender<Proof>>,
}

impl AtlanticProver {
    async fn run(mut self) {
        // TODO: split this type further into sub-services to allow parallelization
        // TODO: add persistence for in-flight proof requests to be able to resume progress

        loop {
            let new_block = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_block = self.block_channel.recv() => new_block,
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

                // TODO: check only for the proof generation job as fact registration doesn't matter
                // TODO: error handling
                let query_status = self
                    .client
                    .get_query_status(&atlantic_query_id)
                    .await
                    .unwrap();

                if query_status == AtlanticQueryStatus::Done {
                    break;
                }
            }

            debug!(
                "Atlantic PIE proof generation finished for query: {}",
                atlantic_query_id
            );

            // TODO: error handling
            let pie_proof = self.client.get_proof(&atlantic_query_id).await.unwrap();

            // TODO: error handling
            let pie_proof: StarkProof = swiftness::parse(pie_proof).unwrap().transform_to();

            info!("Proof generated for block #{}", new_block.number);

            let new_proof = Proof {
                block_number: new_block.number,
                proof: pie_proof,
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

impl AtlanticProverBuilder {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            block_channel: None,
            proof_channel: None,
        }
    }
}

impl ProverBuilder for AtlanticProverBuilder {
    type Prover = AtlanticProver;

    fn build(self) -> Result<Self::Prover> {
        Ok(AtlanticProver {
            client: AtlanticClient::new(self.api_key),
            block_channel: self
                .block_channel
                .ok_or_else(|| anyhow::anyhow!("`block_channel` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn block_channel(mut self, block_channel: Receiver<NewBlock>) -> Self {
        self.block_channel = Some(block_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<Proof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }
}

impl Prover for AtlanticProver {}

impl Daemon for AtlanticProver {
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
