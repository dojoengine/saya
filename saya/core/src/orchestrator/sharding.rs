use crate::{
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    prover::{Prover, ProverBuilder, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    shard::{Aggregator, AggregatorBuilder},
};
use anyhow::Result;
use log::debug;
use swiftness::types::StarkProof;

/// Size of the `NewBlock` channel.
///
/// Block ingestor implementations would typically always make at least one extra block ready to be
/// sent regardless of whether the channel is full. Therefore, setting this value as `1` should be
/// sufficient.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 4;

/// Size of the `StarkProof` channel.
const PROOF_BUFFER_SIZE: usize = 4;

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
pub struct ShardingOrchestrator<I, P, A> {
    ingestor: I,
    prover: P,
    aggregator: A,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct ShardingOrchestratorBuilder<I, P, A> {
    ingestor_builder: I,
    prover_builder: P,
    aggregator_builder: A,
}

struct ShardingOrchestratorState {
    ingestor_handle: ShutdownHandle,
    prover_handle: ShutdownHandle,
    aggregator_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, P, A> ShardingOrchestratorBuilder<I, P, A> {
    pub fn new(ingestor_builder: I, prover_builder: P, aggregator_builder: A) -> Self {
        Self {
            ingestor_builder,
            prover_builder,
            aggregator_builder,
        }
    }
}

impl<I, P, PV, A> ShardingOrchestratorBuilder<I, P, A>
where
    I: BlockIngestorBuilder + Send,
    P: ProverBuilder<Prover = PV> + Send,
    PV: Prover<Statement = BlockInfo, BlockInfo = SnosProof<StarkProof>>,
    A: AggregatorBuilder + Send,
{
    pub async fn build(
        self,
    ) -> Result<ShardingOrchestrator<I::Ingestor, P::Prover, A::Aggregator>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (proof_tx, proof_rx) =
            tokio::sync::mpsc::channel::<SnosProof<StarkProof>>(PROOF_BUFFER_SIZE);
        let start_block = 0;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()
            .unwrap();

        let prover: PV = self
            .prover_builder
            .statement_channel(new_block_rx)
            .proof_channel(proof_tx)
            .build()
            .unwrap();

        let aggregator = self.aggregator_builder.channel(proof_rx).build().unwrap();

        Ok(ShardingOrchestrator {
            ingestor,
            prover,
            aggregator,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl ShardingOrchestratorState {
    async fn run(self) {
        loop {
            // TODO: handle unexpected exit of descendant services
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
            };
        }
        // Request graceful shutdown for all descendant services
        self.ingestor_handle.shutdown();
        self.prover_handle.shutdown();
        self.aggregator_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.prover_handle.finished(),
            self.aggregator_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, P, A> Daemon for ShardingOrchestrator<I, P, A>
where
    I: BlockIngestor + Send,
    P: Prover + Send,
    A: Aggregator + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = ShardingOrchestratorState {
            ingestor_handle: self.ingestor.shutdown_handle(),
            prover_handle: self.prover.shutdown_handle(),
            aggregator_handle: self.aggregator.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.prover.start();
        self.aggregator.start();

        tokio::spawn(state.run());
    }
}
