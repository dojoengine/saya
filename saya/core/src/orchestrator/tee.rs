//! Full TEE orchestrator — wires together all TEE pipeline stages.
//!
//! Pipeline:
//!
//! ```text
//!   BlockIngestor
//!       │ BlockInfo
//!   BlockOrderer           (reorders concurrent ingestor workers into strict sequence)
//!       │ BlockInfo
//!   TeeAttestor            (fetches TEE attestation from Katana)
//!       │ TeeAttestation
//!   OffchainTeeVerifier    (sends attestation to external verifier, receives trace)
//!       │ TeeTrace
//!   TeeProver              (submits trace to TEE proving service, receives proof)
//!       │ TeeProof
//!   adapter task           (wraps proof into DataAvailabilityCursor<BlockInfo>)
//!       │ DataAvailabilityCursor<BlockInfo>
//!   SettlementBackend      (verifies proof on-chain, submits state update)
//!       │ SettlementCursor
//!   TeeOrchestrator loop   (logs confirmed blocks)
//! ```

use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::Receiver;

use crate::{
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    data_availability::DataAvailabilityCursor,
    prover::{
        tee::TeeProof,
        BlockOrderer, BlockOrdererBuilder, PipelineStage, PipelineStageBuilder,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
    tee::{TeeAttestation, TeeTrace},
};

/// Size of the `BlockInfo` channel between ingestor and the orderer.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 4;

/// Size of the `BlockInfo` channel between orderer and the attestor.
const ORDERER_BUFFER_SIZE: usize = 4;

/// Size of the `TeeAttestation` channel between attestor and verifier.
const ATTESTATION_BUFFER_SIZE: usize = 4;

/// Size of the `TeeTrace` channel between verifier and prover.
const TRACE_BUFFER_SIZE: usize = 4;

/// Size of the `TeeProof` channel between prover and the settlement adapter.
const PROOF_BUFFER_SIZE: usize = 4;

/// Size of the `DataAvailabilityCursor` channel fed into settlement.
const DA_CURSOR_BUFFER_SIZE: usize = 4;

/// Size of the `SettlementCursor` channel.
const SETTLE_CURSOR_BUFFER_SIZE: usize = 4;

/// An orchestrator for running a rollup in TEE mode with external proof generation.
///
/// This orchestrator extends [`crate::orchestrator::PersistentTeeOrchestrator`] with an explicit
/// TEE attestation → offchain verification → TEE proving pipeline before settlement.  Each stage
/// is a [`crate::prover::PipelineStage`] connected via bounded async channels.
#[derive(Debug)]
pub struct TeeOrchestrator<I, A, V, P, S> {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor: I,
    orderer: BlockOrderer<BlockInfo>,
    attestor: A,
    verifier: V,
    prover: P,
    settlement: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeOrchestratorBuilder<I, A, V, P, S> {
    ingestor_builder: I,
    attestor_builder: A,
    verifier_builder: V,
    prover_builder: P,
    settlement_builder: S,
}

struct TeeOrchestratorState {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor_handle: ShutdownHandle,
    orderer_handle: ShutdownHandle,
    attestor_handle: ShutdownHandle,
    verifier_handle: ShutdownHandle,
    prover_handle: ShutdownHandle,
    settlement_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, A, V, P, S> TeeOrchestratorBuilder<I, A, V, P, S> {
    pub fn new(
        ingestor_builder: I,
        attestor_builder: A,
        verifier_builder: V,
        prover_builder: P,
        settlement_builder: S,
    ) -> Self {
        Self {
            ingestor_builder,
            attestor_builder,
            verifier_builder,
            prover_builder,
            settlement_builder,
        }
    }
}

impl<I, A, AV, V, VV, P, PV, S> TeeOrchestratorBuilder<I, A, V, P, S>
where
    I: BlockIngestorBuilder + Send,
    A: PipelineStageBuilder<Stage = AV> + Send,
    AV: PipelineStage<Input = BlockInfo, Output = TeeAttestation>,
    V: PipelineStageBuilder<Stage = VV> + Send,
    VV: PipelineStage<Input = TeeAttestation, Output = TeeTrace>,
    P: PipelineStageBuilder<Stage = PV> + Send,
    PV: PipelineStage<Input = TeeTrace, Output = TeeProof>,
    S: SettlementBackendBuilder + Send,
{
    pub async fn build(
        self,
    ) -> Result<TeeOrchestrator<I::Ingestor, AV, VV, PV, S::Backend>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (ordered_tx, ordered_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(ORDERER_BUFFER_SIZE);
        let (attestation_tx, attestation_rx) =
            tokio::sync::mpsc::channel::<TeeAttestation>(ATTESTATION_BUFFER_SIZE);
        let (trace_tx, trace_rx) =
            tokio::sync::mpsc::channel::<TeeTrace>(TRACE_BUFFER_SIZE);
        let (proof_tx, mut proof_rx) =
            tokio::sync::mpsc::channel::<TeeProof>(PROOF_BUFFER_SIZE);
        let (da_cursor_tx, da_cursor_rx) =
            tokio::sync::mpsc::channel::<DataAvailabilityCursor<BlockInfo>>(DA_CURSOR_BUFFER_SIZE);
        let (settle_cursor_tx, settle_cursor_rx) =
            tokio::sync::mpsc::channel::<SettlementCursor>(SETTLE_CURSOR_BUFFER_SIZE);

        let settlement = self
            .settlement_builder
            .da_channel(da_cursor_rx)
            .cursor_channel(settle_cursor_tx)
            .build()
            .await?;

        let start_block = settlement.get_block_number().await? + 1;
        let start_block: u64 = start_block.try_into()?;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()?;

        let orderer = BlockOrdererBuilder::new()
            .start_block(start_block)
            .input_channel(new_block_rx)
            .output_channel(ordered_tx)
            .build()?;

        let attestor = self
            .attestor_builder
            .input_channel(ordered_rx)
            .output_channel(attestation_tx)
            .build()?;

        let verifier = self
            .verifier_builder
            .input_channel(attestation_rx)
            .output_channel(trace_tx)
            .build()?;

        let prover = self
            .prover_builder
            .input_channel(trace_rx)
            .output_channel(proof_tx)
            .build()?;

        // Adapter task: unwraps `TeeProof` back into a `DataAvailabilityCursor<BlockInfo>` for
        // the settlement backend.  The proof bytes should be persisted to the DB here before
        // forwarding so that the settlement backend can retrieve them by block number.
        tokio::spawn(async move {
            while let Some(proof) = proof_rx.recv().await {
                // TODO: persist `proof.data` to the DB (Step::Tee) so the settlement backend
                //   can retrieve it during on-chain TEE proof verification.
                let cursor = DataAvailabilityCursor {
                    block_number: proof.block_info.number,
                    pointer: None,
                    full_payload: proof.block_info,
                };
                if da_cursor_tx.send(cursor).await.is_err() {
                    break;
                }
            }
        });

        Ok(TeeOrchestrator {
            cursor_channel: settle_cursor_rx,
            ingestor,
            orderer,
            attestor,
            verifier,
            prover,
            settlement,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl TeeOrchestratorState {
    async fn run(mut self) {
        loop {
            let new_cursor = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_cursor = self.cursor_channel.recv() => new_cursor,
            };

            let new_cursor = new_cursor.unwrap();

            info!(
                block_number = new_cursor.block_number,
                transaction_hash:% = format!("{:#064x}", new_cursor.transaction_hash);
                "Chain advanced to new block"
            );
        }

        self.ingestor_handle.shutdown();
        self.orderer_handle.shutdown();
        self.attestor_handle.shutdown();
        self.verifier_handle.shutdown();
        self.prover_handle.shutdown();
        self.settlement_handle.shutdown();

        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.orderer_handle.finished(),
            self.attestor_handle.finished(),
            self.verifier_handle.finished(),
            self.prover_handle.finished(),
            self.settlement_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, A, V, P, S> Daemon for TeeOrchestrator<I, A, V, P, S>
where
    I: BlockIngestor + Send,
    A: PipelineStage + Send,
    V: PipelineStage + Send,
    P: PipelineStage + Send,
    S: SettlementBackend + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = TeeOrchestratorState {
            cursor_channel: self.cursor_channel,
            ingestor_handle: self.ingestor.shutdown_handle(),
            orderer_handle: self.orderer.shutdown_handle(),
            attestor_handle: self.attestor.shutdown_handle(),
            verifier_handle: self.verifier.shutdown_handle(),
            prover_handle: self.prover.shutdown_handle(),
            settlement_handle: self.settlement.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.orderer.start();
        self.attestor.start();
        self.verifier.start();
        self.prover.start();
        self.settlement.start();

        tokio::spawn(state.run());
    }
}
