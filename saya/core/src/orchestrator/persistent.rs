use anyhow::Result;
use log::{debug, info};
use tokio::sync::mpsc::Receiver;

use crate::{
    block_ingestor::{BlockIngestor, BlockIngestorBuilder, NewBlock},
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
    },
    prover::{Prover, ProverBuilder, RecursiveProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
};

/// Size of the `NewBlock` channel.
///
/// Block ingestor implementations would typically always make at least one extra block ready to be
/// sent regardless of whether the channel is full. Therefore, setting this value as `1` should be
/// sufficient.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 100;

/// Size of the `StarkProof` channel.
const PROOF_BUFFER_SIZE: usize = 100;

/// Size of the `DataAvailabilityCursor` channel.
const DA_CURSOR_BUFFER_SIZE: usize = 5;

/// Size of the `SettlementCursor` channel.
const SETTLE_CURSOR_BUFFER_SIZE: usize = 5;

/// An orchestrator implementation for running a rollup in persistent mode.
///
/// In this mode, the orchestrator proves blocks and makes full proofs available through a data
/// availability backend. It then applies the state root transition on a settlement layer and
/// publishes the data availability fact simultaneously.
///
/// Notably, the data availability fact is not verified and opaque to the settlement layer.
/// Therefore, with the current implementation, there's a risk that a rollup's sequencer would
/// withhold full state transition data, making it impossible to access the latest state.
#[derive(Debug)]
pub struct PersistentOrchestrator<I, P, D, S> {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor: I,
    prover: P,
    da: D,
    settlement: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PersistentOrchestratorBuilder<I, P, D, S> {
    ingestor_builder: I,
    prover_builder: P,
    da_builder: D,
    settlement_builder: S,
}

struct PersistentOrchestratorState {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor_handle: ShutdownHandle,
    prover_handle: ShutdownHandle,
    da_handle: ShutdownHandle,
    settlement_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, P, D, S> PersistentOrchestratorBuilder<I, P, D, S> {
    pub fn new(
        ingestor_builder: I,
        prover_builder: P,
        da_builder: D,
        settlement_builder: S,
    ) -> Self {
        Self {
            ingestor_builder,
            prover_builder,
            da_builder,
            settlement_builder,
        }
    }
}

impl<I, P, PV, D, DB, S> PersistentOrchestratorBuilder<I, P, D, S>
where
    I: BlockIngestorBuilder + Send,
    P: ProverBuilder<Prover = PV> + Send,
    PV: Prover<Statement = NewBlock, Proof = RecursiveProof>,
    D: DataAvailabilityBackendBuilder<Backend = DB> + Send,
    DB: DataAvailabilityBackend<Payload = RecursiveProof>,
    S: SettlementBackendBuilder + Send,
{
    pub async fn build(
        self,
    ) -> Result<PersistentOrchestrator<I::Ingestor, P::Prover, D::Backend, S::Backend>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<NewBlock>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (proof_tx, proof_rx) = tokio::sync::mpsc::channel::<RecursiveProof>(PROOF_BUFFER_SIZE);
        let (da_cursor_tx, da_cursor_rx) = tokio::sync::mpsc::channel::<
            DataAvailabilityCursor<RecursiveProof>,
        >(DA_CURSOR_BUFFER_SIZE);
        let (settle_cursor_tx, settle_cursor_rx) =
            tokio::sync::mpsc::channel::<SettlementCursor>(SETTLE_CURSOR_BUFFER_SIZE);

        let settlement = self
            .settlement_builder
            .da_channel(da_cursor_rx)
            .cursor_channel(settle_cursor_tx)
            .build()
            .await
            .unwrap();

        // Since the `Felt` type is wrapping (`Felt::MAX + 1 = 0`), there is not
        // need for a special case for the genesis block, and `+1` works as expected.
        //
        // TODO: should we change to `settlement.next_block_number()` instead to always return `u64`?
        let start_block = settlement.get_block_number().await? + 1;

        // Now that the special value of `Felt::MAX` is handled, we can use the block number as `u64`.
        let start_block: u64 = start_block.try_into()?;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()
            .unwrap();

        let prover = self
            .prover_builder
            .statement_channel(new_block_rx)
            .proof_channel(proof_tx)
            .build()
            .unwrap();

        let da = self
            .da_builder
            .proof_channel(proof_rx)
            .cursor_channel(da_cursor_tx)
            .build()
            .unwrap();

        Ok(PersistentOrchestrator {
            cursor_channel: settle_cursor_rx,
            ingestor,
            prover,
            da,
            settlement,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl PersistentOrchestratorState {
    async fn run(mut self) {
        loop {
            // TODO: handle unexpected exit of descendant services
            let new_cursor = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_cursor = self.cursor_channel.recv() => new_cursor,
            };

            // This should be fine for now as da backends wouldn't drop senders. This might change
            // in the future.
            let new_cursor = new_cursor.unwrap();

            info!(
                "Chain advanced to block #{} (settled with tx: {:#064x})",
                new_cursor.block_number, new_cursor.transaction_hash
            );
        }

        // Request graceful shutdown for all descendant services
        self.ingestor_handle.shutdown();
        self.prover_handle.shutdown();
        self.da_handle.shutdown();
        self.settlement_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.prover_handle.finished(),
            self.da_handle.finished(),
            self.settlement_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, P, D, S> Daemon for PersistentOrchestrator<I, P, D, S>
where
    I: BlockIngestor + Send,
    P: Prover + Send,
    D: DataAvailabilityBackend + Send,
    S: SettlementBackend + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = PersistentOrchestratorState {
            cursor_channel: self.cursor_channel,
            ingestor_handle: self.ingestor.shutdown_handle(),
            prover_handle: self.prover.shutdown_handle(),
            da_handle: self.da.shutdown_handle(),
            settlement_handle: self.settlement.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.prover.start();
        self.da.start();
        self.settlement.start();

        tokio::spawn(state.run());
    }
}
