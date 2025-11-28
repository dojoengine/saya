use anyhow::Result;
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{
        AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder, MockLayoutBridgeProver,
        MockLayoutBridgeProverBuilder, Prover, ProverBuilder, SnosProof,
    },
    service::{Daemon, ShutdownHandle},
    storage::PersistantStorage,
};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub enum AnyLayoutBridgeProver<DB> {
    Atlantic(AtlanticLayoutBridgeProver<DB>),
    Mock(MockLayoutBridgeProver<DB>),
}

#[derive(Debug)]
pub enum AnyLayoutBridgeProverBuilder<DB> {
    Atlantic(AtlanticLayoutBridgeProverBuilder<DB>),
    Mock(MockLayoutBridgeProverBuilder<DB>),
}

impl<DB> Prover for AnyLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Statement = SnosProof<String>;
    type BlockInfo = BlockInfo;
}

impl<DB> Daemon for AnyLayoutBridgeProver<DB>
where
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

impl<DB> ProverBuilder for AnyLayoutBridgeProverBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Prover = AnyLayoutBridgeProver<DB>;

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

    fn proof_channel(self, proof_channel: Sender<<Self::Prover as Prover>::BlockInfo>) -> Self {
        match self {
            Self::Atlantic(inner) => Self::Atlantic(inner.proof_channel(proof_channel)),
            Self::Mock(inner) => Self::Mock(inner.proof_channel(proof_channel)),
        }
    }
}
