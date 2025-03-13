use anyhow::Result;
use log::debug;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    prover::{Prover, ProverBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

const BRIDGE_BUFFER_SIZE: usize = 1;

#[derive(Debug)]
pub struct RecursiveProver<U, D> {
    upstream_prover: U,
    downstream_prover: D,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct RecursiveProverBuilder<U, D> {
    upstream_prover_builder: U,
    downstream_prover_builder: D,
}

struct RecursiveProverState {
    upstream_prover_handle: ShutdownHandle,
    downstream_prover_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<U, D> RecursiveProverBuilder<U, D> {
    pub fn new(upstream_prover_builder: U, downstream_prover_builder: D) -> Self {
        Self {
            upstream_prover_builder,
            downstream_prover_builder,
        }
    }
}

impl<U, D, UV, DV, I> ProverBuilder for RecursiveProverBuilder<U, D>
where
    U: ProverBuilder<Prover = UV>,
    D: ProverBuilder<Prover = DV>,
    UV: Prover<BlockInfo = I>,
    DV: Prover<Statement = I>,
{
    type Prover = RecursiveProver<U::Prover, D::Prover>;

    fn build(self) -> Result<Self::Prover> {
        let (bridge_tx, bridge_rx) = tokio::sync::mpsc::channel::<I>(BRIDGE_BUFFER_SIZE);

        Ok(RecursiveProver {
            upstream_prover: self
                .upstream_prover_builder
                .proof_channel(bridge_tx)
                .build()?,
            downstream_prover: self
                .downstream_prover_builder
                .statement_channel(bridge_rx)
                .build()?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn statement_channel(
        self,
        block_channel: Receiver<<Self::Prover as Prover>::Statement>,
    ) -> Self {
        Self {
            upstream_prover_builder: self
                .upstream_prover_builder
                .statement_channel(block_channel),
            downstream_prover_builder: self.downstream_prover_builder,
        }
    }

    fn proof_channel(self, proof_channel: Sender<<Self::Prover as Prover>::BlockInfo>) -> Self {
        Self {
            upstream_prover_builder: self.upstream_prover_builder,
            downstream_prover_builder: self.downstream_prover_builder.proof_channel(proof_channel),
        }
    }
}

impl RecursiveProverState {
    async fn run(self) {
        self.finish_handle.shutdown_requested().await;

        // Request graceful shutdown for all descendant services
        self.upstream_prover_handle.shutdown();
        self.downstream_prover_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.upstream_prover_handle.finished(),
            self.downstream_prover_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<U, D, I> Prover for RecursiveProver<U, D>
where
    U: Prover<BlockInfo = I>,
    D: Prover<Statement = I>,
{
    type Statement = U::Statement;
    type BlockInfo = D::BlockInfo;
}

impl<U, D, I> Daemon for RecursiveProver<U, D>
where
    U: Prover<BlockInfo = I>,
    D: Prover<Statement = I>,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = RecursiveProverState {
            upstream_prover_handle: self.upstream_prover.shutdown_handle(),
            downstream_prover_handle: self.downstream_prover.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.upstream_prover.start();
        self.downstream_prover.start();

        tokio::spawn(state.run());
    }
}
