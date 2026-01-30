use crate::block_ingestor::BlockInfo;
use anyhow::Result;
use integrity::Felt;
use log::{debug, info};
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::storage::PersistantStorage;
use crate::{
    prover::{Prover, ProverBuilder, RecursiveProof, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    utils::calculate_output,
};
/// Prover implementation as a client to the hosted [Mock Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct MockLayoutBridgeProver<DB> {
    statement_channel: Receiver<SnosProof<String>>,
    block_info_channel: Sender<BlockInfo>,
    layout_bridge_program_hash: Felt,
    finish_handle: FinishHandle,
    db: DB,
}

#[derive(Debug, Default)]
pub struct MockLayoutBridgeProverBuilder<DB> {
    statement_channel: Option<Receiver<SnosProof<String>>>,
    block_info_channel: Option<Sender<BlockInfo>>,
    layout_bridge_program_hash: Felt,
    db: DB,
}

impl<DB> MockLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn run(mut self) {
        loop {
            let new_snos_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_snos_proof = self.statement_channel.recv() => new_snos_proof,
            };
            // This should be fine for now as block ingestors wouldn't drop senders. This might
            // change in the future.
            let new_snos_proof = new_snos_proof.unwrap();
            let state_update = self
                .db
                .get_state_update(new_snos_proof.block_number.try_into().unwrap())
                .await
                .unwrap();

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

            let mut bootloader_output = vec![
                // Bootloader constants (number of task executed by bootloader, in case of herodotus its always 1)
                Felt::ONE,
                // Bootloader output len (not checked by piltover, set to 0)
                Felt::ZERO,
                // Verifier Program Hash (not checked by piltover, set to 0)
                self.layout_bridge_program_hash,
                // Bootloader program hash
                Felt::from_hex_unchecked(
                    "0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07",
                ),
                // Verifier output len (not checked by piltover, set to 0)
                Felt::ZERO,
            ];
            bootloader_output.extend_from_slice(&snos_output);

            let mock_proof = crate::utils::stark_proof_mock(&bootloader_output);

            let string_proof = serde_json::to_string(&mock_proof).unwrap();
            let bytes_proof = string_proof.as_bytes();

            self.db
                .add_proof(
                    new_snos_proof.block_number.try_into().unwrap(),
                    bytes_proof.to_vec(),
                    crate::storage::Step::Bridge,
                )
                .await
                .unwrap();
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
                state_update: Some(state_update),
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

impl<DB> MockLayoutBridgeProverBuilder<DB> {
    pub fn new(layout_bridge_program_hash: Felt, db: DB) -> Self {
        Self {
            statement_channel: None,
            block_info_channel: None,
            layout_bridge_program_hash,
            db,
        }
    }
}

impl<DB> ProverBuilder for MockLayoutBridgeProverBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Prover = MockLayoutBridgeProver<DB>;

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
            db: self.db,
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

impl<DB> Prover for MockLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Statement = SnosProof<String>;
    type BlockInfo = BlockInfo;
}

impl<DB> Daemon for MockLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
