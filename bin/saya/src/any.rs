use anyhow::Result;
use saya_core::{
    prover::{
        AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder, MockLayoutBridgeProver,
        MockLayoutBridgeProverBuilder, Prover, ProverBuilder, RecursiveProof, SnosProof,
    },
    service::{Daemon, ShutdownHandle},
};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub enum AnyLayoutBridgeProver {
    Atlantic(AtlanticLayoutBridgeProver),
    Mock(MockLayoutBridgeProver),
}

#[derive(Debug)]
pub enum AnyLayoutBridgeProverBuilder {
    Atlantic(AtlanticLayoutBridgeProverBuilder),
    Mock(MockLayoutBridgeProverBuilder),
}

impl Prover for AnyLayoutBridgeProver {
    type Statement = SnosProof<String>;
    type Proof = RecursiveProof;
}

impl Daemon for AnyLayoutBridgeProver {
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

impl ProverBuilder for AnyLayoutBridgeProverBuilder {
    type Prover = AnyLayoutBridgeProver;

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
