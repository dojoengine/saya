use anyhow::Result;
use saya_core::{
    prover::{
        AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder, LayoutBridgeTraceGenerator,
        MockLayoutBridgeProver, MockLayoutBridgeProverBuilder, Prover, ProverBuilder,
        RecursiveProof, SnosProof,
    },
    service::{Daemon, ShutdownHandle},
    storage::PersistantStorage,
};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub enum AnyLayoutBridgeProver<T, DB> {
    Atlantic(AtlanticLayoutBridgeProver<T, DB>),
    Mock(MockLayoutBridgeProver),
}

#[derive(Debug)]
pub enum AnyLayoutBridgeProverBuilder<T, DB> {
    Atlantic(AtlanticLayoutBridgeProverBuilder<T, DB>),
    Mock(MockLayoutBridgeProverBuilder),
}

impl<T, DB> Prover for AnyLayoutBridgeProver<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl<T, DB> Daemon for AnyLayoutBridgeProver<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        match self {
            Self::Atlantic(inner) => inner.shutdown_handle(),
            Self::Mock(inner) => inner.shutdown_handle(),
        }
    }

    fn start(self) {
        match self {
            Self::Atlantic(inner) => inner.start(),
            Self::Mock(inner) => inner.start(),
        }
    }
}

impl<T, DB> ProverBuilder for AnyLayoutBridgeProverBuilder<T, DB>
where
    T: LayoutBridgeTraceGenerator<DB> + Send + Sync + Clone + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Prover = AnyLayoutBridgeProver<T, DB>;

    fn build(self) -> Result<Self::Prover> {
        Ok(match self {
            Self::Atlantic(inner) => AnyLayoutBridgeProver::Atlantic(inner.build()?),
            Self::Mock(inner) => AnyLayoutBridgeProver::Mock(inner.build()?),
        })
    }

    fn statement_channel(
        self,
        block_channel: Receiver<<Self::Prover as Prover>::Statement>,
    ) -> Self {
        match self {
            Self::Atlantic(inner) => Self::Atlantic(inner.statement_channel(block_channel)),
            Self::Mock(inner) => Self::Mock(inner.statement_channel(block_channel)),
        }
    }

    fn proof_channel(self, proof_channel: Sender<<Self::Prover as Prover>::Proof>) -> Self {
        match self {
            Self::Atlantic(inner) => Self::Atlantic(inner.proof_channel(proof_channel)),
            Self::Mock(inner) => Self::Mock(inner.proof_channel(proof_channel)),
        }
    }
}
