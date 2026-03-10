//! TEE prover — submits an execution trace to a TEE proving service and retrieves the resulting
//! proof.
//!
//! The prover posts the [`TeeTrace`] to a remote proving service, polls for completion, and
//! emits a [`TeeProof`] downstream for verification and settlement.

use crate::prover_impl::TeeAttestation as TeeAttestationProver;
use anyhow::Result;
use katana_tee_client::ProverConfig;
use katana_tee_client::TeeQuoteResponse;
use log::{debug, info};
use saya_core::{
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder, TeeProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::TeeAttestation,
};
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};

/// Submits a [`TeeAttestation`] to the TEE proving service and emits the resulting [`TeeProof`].
#[derive(Debug)]
pub struct TeeProver {
    provider_url: String,
    registry_address: Felt,
    private_key: String,
    input_channel: Receiver<TeeAttestation>,
    output_channel: Sender<TeeProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeProverBuilder {
    provider_url: String,
    registry_address: Felt,
    private_key: String,
    input_channel: Option<Receiver<TeeAttestation>>,
    output_channel: Option<Sender<TeeProof>>,
}

impl TeeProverBuilder {
    pub fn new(provider_url: String, registry_address: Felt, private_key: String) -> Self {
        Self {
            provider_url,
            registry_address,
            private_key,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeProverBuilder {
    type Stage = TeeProver;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeProver {
            provider_url: self.provider_url,
            registry_address: self.registry_address,
            private_key: self.private_key,
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

    /// Submits the trace to the TEE proving service and polls until the proof is ready.
    ///
    /// TODO: implement the actual prover HTTP calls:
    ///   1. POST `{prover_url}/prove` with the raw trace bytes → receive a `job_id`.
    ///   2. Poll GET `{prover_url}/result/{job_id}` until the job is complete.
    ///   3. Fetch and return the raw proof bytes.
    async fn prove(&self, attestation: TeeAttestation) -> Result<TeeProof> {
        let block_number = attestation.block_number();

        info!(block_number; "TEE proving started for block batch");
        let response = TeeQuoteResponse {
            quote: attestation.quote.clone(),
            prev_state_root: attestation.prev_state_root.clone(),
            state_root: attestation.state_root.clone(),
            prev_block_hash: attestation.prev_block_hash.clone(),
            block_hash: attestation.block_hash.clone(),
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            events_commitment: None,
        };

        let tee = TeeAttestationProver::from_response(&response)?;
        let config = ProverConfig {
            rpc_url: None,
            private_key: Some(self.private_key.clone()),
            skip_time_validity_check: false,
        };
        let proof = tee
            .generate_proof(&self.provider_url, self.registry_address, config)
            .await?;
        let proof_raw = proof.encode_json()?;
        info!(block_number; "TEE proving completed, proof size: {} bytes", proof_raw.len());

        let prev_state_root = Felt::from_hex_unchecked(&attestation.prev_state_root);
        let state_root = Felt::from_hex(&attestation.state_root)?;
        let prev_block_hash = Felt::from_hex(&attestation.prev_block_hash)?;
        let block_hash = Felt::from_hex(&attestation.block_hash)?;

        Ok(TeeProof {
            blocks: attestation.blocks,
            data: proof_raw,
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
        })
    }
}
