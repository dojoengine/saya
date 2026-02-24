//! TEE prover — submits an execution trace to a TEE proving service and retrieves the resulting
//! proof.
//!
//! The prover posts the [`TeeTrace`] to a remote proving service, polls for completion, and
//! emits a [`TeeProof`] downstream for verification and settlement.

use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    block_ingestor::BlockInfo,
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::TeeTrace,
};

/// A proof produced by the TEE proving service for a single block.
///
/// The `block_info` is carried from the original [`crate::block_ingestor::BlockInfo`] so the
/// settlement adapter can construct a [`crate::data_availability::DataAvailabilityCursor`]
/// without an extra DB lookup.
///
/// TODO: derive `Serialize`/`Deserialize` once the proof format is defined (the `block_info`
/// field would be stored separately or reconstructed from context).
#[derive(Debug, Clone)]
pub struct TeeProof {
    /// Original block info, carried through the pipeline.
    pub block_info: BlockInfo,
    /// Raw proof bytes returned by the TEE proving service.
    ///
    /// TODO: define concrete proof format once the prover API is stable.
    pub data: Vec<u8>,
}

impl HasBlockNumber for TeeProof {
    fn block_number(&self) -> u64 {
        self.block_info.number
    }
}

/// Submits a [`TeeTrace`] to the TEE proving service and emits the resulting [`TeeProof`].
#[derive(Debug)]
pub struct TeeProver {
    prover_url: Url,
    input_channel: Receiver<TeeTrace>,
    output_channel: Sender<TeeProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeProverBuilder {
    prover_url: Url,
    input_channel: Option<Receiver<TeeTrace>>,
    output_channel: Option<Sender<TeeProof>>,
}

impl TeeProverBuilder {
    pub fn new(prover_url: Url) -> Self {
        Self {
            prover_url,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeProverBuilder {
    type Stage = TeeProver;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeProver {
            prover_url: self.prover_url,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<TeeTrace>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeProof>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeProver {
    type Input = TeeTrace;
    type Output = TeeProof;
}

impl Daemon for TeeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeProver {
    async fn run(mut self) {
        loop {
            let trace = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                t = self.input_channel.recv() => match t {
                    Some(t) => t,
                    None => break,
                },
            };

            debug!(block_number = trace.block_number(); "Submitting TEE trace to prover");

            let proof = match self.prove(trace).await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("TEE proof generation failed: {}", e);
                    continue;
                }
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(proof) => {},
            }
        }

        debug!("TeeProver graceful shutdown finished");
        self.finish_handle.finish();
    }

    /// Submits the trace to the TEE proving service and polls until the proof is ready.
    ///
    /// TODO: implement the actual prover HTTP calls:
    ///   1. POST `{prover_url}/prove` with the raw trace bytes → receive a `job_id`.
    ///   2. Poll GET `{prover_url}/result/{job_id}` until the job is complete.
    ///   3. Fetch and return the raw proof bytes.
    async fn prove(&self, trace: TeeTrace) -> Result<TeeProof> {
        let block_number = trace.block_number();

        // TODO: replace with a real HTTP call to the TEE proving service.
        info!(block_number; "TEE proving not yet implemented — returning empty placeholder");

        Ok(TeeProof {
            block_info: trace.block_info,
            data: vec![],
        })
    }
}

