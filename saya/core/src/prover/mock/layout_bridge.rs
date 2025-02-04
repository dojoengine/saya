use std::borrow::Cow;

use anyhow::Result;
use log::{debug, info};
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    prover::{
        mock::stark_proof_mock::StarkProofMockBuilder, Prover, ProverBuilder, RecursiveProof, SnosProof
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};

/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct LayoutBridgeMockProver {
    statement_channel: Receiver<SnosProof<String>>,
    proof_channel: Sender<RecursiveProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct LayoutBridgeMockProverBuilder {
    statement_channel: Option<Receiver<SnosProof<String>>>,
    proof_channel: Option<Sender<RecursiveProof>>,
}

impl LayoutBridgeMockProver {
    async fn run(mut self) {
        loop {
            let new_snos_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_snos_proof = self.statement_channel.recv() => new_snos_proof,
            };

            // This should be fine for now as block ingestors wouldn't drop senders. This might
            // change in the future.
            let new_snos_proof = new_snos_proof.unwrap();

            debug!(
                "Receive raw SNOS proof for block #{}",
                new_snos_proof.block_number
            );

            // TODO: error handling
            let parsed_snos_proof: StarkProof = swiftness::parse(&new_snos_proof.proof)
                .unwrap()
                .transform_to();

            info!("Proof mocked for block #{}", new_snos_proof.block_number);

            let snos_output = calculate_output(&parsed_snos_proof);

            let layout_bridge_proof = StarkProof::mock_from_output(&snos_output);

            let new_proof = RecursiveProof {
                block_number: new_snos_proof.block_number,
                snos_output,
                layout_bridge_proof,
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

impl LayoutBridgeMockProverBuilder {
    pub fn new<P>() -> Self
    where
        P: Into<Cow<'static, [u8]>>,
    {
        Self {
            statement_channel: None,
            proof_channel: None,
        }
    }
}

impl ProverBuilder for LayoutBridgeMockProverBuilder {
    type Prover = LayoutBridgeMockProver;

    fn build(self) -> Result<Self::Prover> {
        Ok(LayoutBridgeMockProver {
            statement_channel: self
                .statement_channel
                .ok_or_else(|| anyhow::anyhow!("`statement_channel` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn statement_channel(mut self, statement_channel: Receiver<SnosProof<String>>) -> Self {
        self.statement_channel = Some(statement_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<RecursiveProof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }
}

impl Prover for LayoutBridgeMockProver {
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl Daemon for LayoutBridgeMockProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
