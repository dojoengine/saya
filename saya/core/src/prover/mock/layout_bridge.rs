use crate::block_ingestor::BlockInfo;
use anyhow::Result;
use integrity::Felt;
use log::{debug, info};
use starknet_crypto::poseidon_hash_many;
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    prover::{Prover, ProverBuilder, RecursiveProof, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};

/// Prover implementation as a client to the hosted [Mock Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct MockLayoutBridgeProver {
    statement_channel: Receiver<SnosProof<String>>,
    block_info_channel: Sender<BlockInfo>,
    layout_bridge_program_hash: Felt,
    finish_handle: FinishHandle,
}

#[derive(Debug, Default)]
pub struct MockLayoutBridgeProverBuilder {
    statement_channel: Option<Receiver<SnosProof<String>>>,
    block_info_channel: Option<Sender<BlockInfo>>,
    layout_bridge_program_hash: Felt,
}

impl MockLayoutBridgeProver {
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
            let parsed_snos_proof: StarkProof = match swiftness::parse(&new_snos_proof.proof) {
                Ok(proof) => proof.transform_to(),
                Err(_) => {
                    // If the proof is sent by a mocked SNOS, it's already in the correct format.
                    serde_json::from_str::<StarkProof>(&new_snos_proof.proof).unwrap()
                }
            };

            let snos_output = calculate_output(&parsed_snos_proof);

            let bootloader_output = [
                Felt::ZERO,
                Felt::ZERO,
                self.layout_bridge_program_hash,
                Felt::ZERO,
                poseidon_hash_many(&snos_output),
            ];

            let mock_proof = crate::utils::stark_proof_mock(&bootloader_output);

            let new_proof = RecursiveProof {
                block_number: new_snos_proof.block_number,
                snos_output,
                layout_bridge_proof: mock_proof,
            };

            info!(
                "Mock proof generated for block #{}",
                new_snos_proof.block_number
            );
            let new_proof = BlockInfo {
                number: new_proof.block_number,
                status: crate::storage::BlockStatus::BridgeProofGenerated,
            };
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.block_info_channel.send(new_proof) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl MockLayoutBridgeProverBuilder {
    pub fn new(layout_bridge_program_hash: Felt) -> Self {
        Self {
            statement_channel: None,
            block_info_channel: None,
            layout_bridge_program_hash,
        }
    }
}

impl ProverBuilder for MockLayoutBridgeProverBuilder {
    type Prover = MockLayoutBridgeProver;

    fn build(self) -> Result<Self::Prover> {
        Ok(MockLayoutBridgeProver {
            statement_channel: self
                .statement_channel
                .ok_or_else(|| anyhow::anyhow!("`statement_channel` not set"))?,
            block_info_channel: self
                .block_info_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            layout_bridge_program_hash: self.layout_bridge_program_hash,
        })
    }

    fn statement_channel(mut self, statement_channel: Receiver<SnosProof<String>>) -> Self {
        self.statement_channel = Some(statement_channel);
        self
    }

    fn proof_channel(mut self, proof_channel: Sender<BlockInfo>) -> Self {
        self.block_info_channel = Some(proof_channel);
        self
    }
}

impl Prover for MockLayoutBridgeProver {
    type Statement = SnosProof<String>;
    type BlockInfo = BlockInfo;
}

impl Daemon for MockLayoutBridgeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
