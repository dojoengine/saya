//! TEE prover — submits an execution trace to a TEE proving service and retrieves the resulting
//! proof.
//!
//! The prover posts the [`TeeTrace`] to a remote proving service, polls for completion, and
//! emits a [`TeeProof`] downstream for verification and settlement.

use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::{Receiver, Sender};

use starknet_types_core::felt::Felt;

use crate::{
    block_ingestor::BlockInfo,
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::{L1ToL2Message, L2ToL1Message, TeeAttestation},
};

/// A proof produced by the TEE proving service for a batch of blocks.
///
/// Carries both the raw proof bytes and the block state fields needed to build the
/// [`piltover::TEEInput`] for `update_state` without an extra DB lookup.
#[derive(Debug, Clone)]
pub struct TeeProof {
    /// Ordered batch of blocks covered by this proof, carried through the pipeline.
    pub blocks: Vec<BlockInfo>,
    /// JSON-encoded `OnchainProof` returned by the TEE proving service.
    pub data: Vec<u8>,
    // Block state fields carried from TeeAttestation for Piltover TEEInput construction.
    pub prev_state_root: Felt,
    pub state_root: Felt,
    pub prev_block_hash: Felt,
    pub block_hash: Felt,
    pub prev_block_number: Felt,
    pub block_number: Felt,
    /// Poseidon commitment over all L1↔L2 messages in the attested block range.
    pub messages_commitment: Felt,
    /// All L2→L1 messages emitted in the attested block range.
    pub l2_to_l1_messages: Vec<L2ToL1Message>,
    /// All L1→L2 messages processed in the attested block range.
    pub l1_to_l2_messages: Vec<L1ToL2Message>,
}

impl HasBlockNumber for TeeProof {
    /// Returns the block number of the last block in the batch — used for pipeline ordering.
    fn block_number(&self) -> u64 {
        self.blocks.last().expect("non-empty proof batch").number
    }
}

/// Submits a [`TeeTrace`] to the TEE proving service and emits the resulting [`TeeProof`].
#[derive(Debug)]
pub struct TeeProver {
    input_channel: Receiver<TeeAttestation>,
    output_channel: Sender<TeeProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug, Default)]
pub struct TeeProverBuilder {
    input_channel: Option<Receiver<TeeAttestation>>,
    output_channel: Option<Sender<TeeProof>>,
}

impl TeeProverBuilder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PipelineStageBuilder for TeeProverBuilder {
    type Stage = TeeProver;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeProver {
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<TeeAttestation>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeProof>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeProver {
    type Input = TeeAttestation;
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

            debug!(block_number = trace.block_number(); "Submitting TEE attestation to prover");

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

    /// Submits the attestation to the TEE proving service and polls until the proof is ready.
    ///
    /// TODO: implement the actual prover HTTP calls:
    ///   1. POST `{prover_url}/prove` with the raw attestation bytes → receive a `job_id`.
    ///   2. Poll GET `{prover_url}/result/{job_id}` until the job is complete.
    ///   3. Fetch and return the raw proof bytes.
    async fn prove(&self, attestation: TeeAttestation) -> Result<TeeProof> {
        let block_number = attestation.block_number();

        // TODO: replace with a real HTTP call to the TEE proving service.
        info!(block_number; "TEE proving not yet implemented — returning empty placeholder");

        let prev_state_root = Felt::from_hex(&attestation.prev_state_root)?;
        let state_root = Felt::from_hex(&attestation.state_root)?;
        let prev_block_hash = Felt::from_hex(&attestation.prev_block_hash)?;
        let block_hash = Felt::from_hex(&attestation.block_hash)?;

        Ok(TeeProof {
            blocks: attestation.blocks,
            data: vec![],
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages: attestation.l2_to_l1_messages,
            l1_to_l2_messages: attestation.l1_to_l2_messages,
        })
    }
}
