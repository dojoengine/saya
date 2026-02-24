//! TEE attestor — fetches attestation data from the Katana rollup node.
//!
//! The attestor polls the Katana JSON-RPC on a configurable interval until the attestation for
//! the given block is available, then emits a [`TeeAttestation`] downstream.

use std::time::Duration;

use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::TeeAttestation,
};

/// Fetches TEE attestation from the Katana rollup node for each incoming [`BlockInfo`].
///
/// Blocks arrive in order from the upstream [`crate::prover::BlockOrderer`].  For each block the
/// attestor calls the Katana attestation endpoint and retries at `poll_interval` until a result is
/// available.
#[derive(Debug)]
pub struct TeeAttestor {
    katana_rpc: Url,
    poll_interval: Duration,
    input_channel: Receiver<BlockInfo>,
    output_channel: Sender<TeeAttestation>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeAttestorBuilder {
    katana_rpc: Url,
    poll_interval: Duration,
    input_channel: Option<Receiver<BlockInfo>>,
    output_channel: Option<Sender<TeeAttestation>>,
}

impl TeeAttestorBuilder {
    pub fn new(katana_rpc: Url, poll_interval: Duration) -> Self {
        Self {
            katana_rpc,
            poll_interval,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeAttestorBuilder {
    type Stage = TeeAttestor;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeAttestor {
            katana_rpc: self.katana_rpc,
            poll_interval: self.poll_interval,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<BlockInfo>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeAttestation>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeAttestor {
    type Input = BlockInfo;
    type Output = TeeAttestation;
}

impl Daemon for TeeAttestor {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeAttestor {
    async fn run(mut self) {
        loop {
            let block = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                block = self.input_channel.recv() => match block {
                    Some(b) => b,
                    None => break,
                },
            };

            debug!(block_number = block.number; "Fetching TEE attestation");

            let attestation = match self.fetch_attestation(block).await {
                Ok(a) => a,
                Err(e) => {
                    log::error!("Failed to fetch TEE attestation: {}", e);
                    continue;
                }
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(attestation) => {},
            }
        }

        debug!("TeeAttestor graceful shutdown finished");
        self.finish_handle.finish();
    }

    /// Polls the Katana attestation endpoint until the attestation for `block` is available.
    ///
    /// TODO: implement the actual Katana TEE attestation RPC call.
    ///   Expected: POST/GET `{katana_rpc}/tee/attest` with `block_number` → raw attestation bytes.
    async fn fetch_attestation(&self, block: BlockInfo) -> Result<TeeAttestation> {
        let block_number = block.number;

        // TODO: replace with a real HTTP/JSON-RPC call to the Katana attestation endpoint.
        //   Retry on `self.poll_interval` until the attestation is available.
        info!(block_number; "TEE attestation fetch not yet implemented — returning empty placeholder");

        Ok(TeeAttestation {
            block_info: block,
            raw: vec![],
        })
    }
}
