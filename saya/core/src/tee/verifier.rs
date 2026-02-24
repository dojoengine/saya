//! Offchain TEE verifier — submits a [`TeeAttestation`] to an external verifier service and
//! retrieves the resulting [`TeeTrace`] that feeds into the TEE prover.

use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::{TeeAttestation, TeeTrace},
};

/// Forwards each [`TeeAttestation`] to an external offchain verifier service and emits the
/// resulting [`TeeTrace`] for the TEE prover.
#[derive(Debug)]
pub struct OffchainTeeVerifier {
    verifier_url: Url,
    input_channel: Receiver<TeeAttestation>,
    output_channel: Sender<TeeTrace>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct OffchainTeeVerifierBuilder {
    verifier_url: Url,
    input_channel: Option<Receiver<TeeAttestation>>,
    output_channel: Option<Sender<TeeTrace>>,
}

impl OffchainTeeVerifierBuilder {
    pub fn new(verifier_url: Url) -> Self {
        Self {
            verifier_url,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for OffchainTeeVerifierBuilder {
    type Stage = OffchainTeeVerifier;

    fn build(self) -> Result<Self::Stage> {
        Ok(OffchainTeeVerifier {
            verifier_url: self.verifier_url,
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

    fn output_channel(mut self, output_channel: Sender<TeeTrace>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for OffchainTeeVerifier {
    type Input = TeeAttestation;
    type Output = TeeTrace;
}

impl Daemon for OffchainTeeVerifier {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl OffchainTeeVerifier {
    async fn run(mut self) {
        loop {
            let attestation = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                a = self.input_channel.recv() => match a {
                    Some(a) => a,
                    None => break,
                },
            };

            debug!(block_number = attestation.block_number(); "Sending TEE attestation to offchain verifier");

            let trace = match self.verify(attestation).await {
                Ok(t) => t,
                Err(e) => {
                    log::error!("Offchain TEE verification failed: {}", e);
                    continue;
                }
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(trace) => {},
            }
        }

        debug!("OffchainTeeVerifier graceful shutdown finished");
        self.finish_handle.finish();
    }

    /// Sends the attestation bytes to the external verifier service and returns the trace.
    ///
    /// TODO: implement the actual HTTP call to the offchain TEE verifier.
    ///   Expected flow:
    ///     1. POST `{verifier_url}/verify` with the raw attestation bytes.
    ///     2. Poll or await the response until the trace is ready.
    ///     3. Return the raw trace bytes.
    async fn verify(&self, attestation: TeeAttestation) -> Result<TeeTrace> {
        let block_number = attestation.block_number();

        // TODO: replace with a real HTTP call to the offchain TEE verifier service.
        info!(block_number; "Offchain TEE verification not yet implemented — returning empty placeholder");

        Ok(TeeTrace {
            block_info: attestation.block_info,
            data: vec![],
        })
    }
}
