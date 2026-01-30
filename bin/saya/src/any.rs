use anyhow::Result;
use saya_core::{
    block_ingestor::BlockInfo,
    data_availability::{
        CelestiaDataAvailabilityBackend, CelestiaDataAvailabilityBackendBuilder,
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
        DataAvailabilityPayload, DataAvailabilityPointer, NoopDataAvailabilityBackend,
        NoopDataAvailabilityBackendBuilder,
    },
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

#[derive(Debug)]
pub enum AnyDataAvailabilityLayer<P> {
    Celestia(Box<CelestiaDataAvailabilityBackend<P>>),
    Noop(NoopDataAvailabilityBackend<P>),
}

#[derive(Debug)]
pub enum AnyDataAvailabilityLayerBuilder<P> {
    Celestia(Box<CelestiaDataAvailabilityBackendBuilder<P>>),
    Noop(NoopDataAvailabilityBackendBuilder<P>),
}

impl<P> DataAvailabilityBackend for AnyDataAvailabilityLayer<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Payload = P;
}

impl<P> Daemon for AnyDataAvailabilityLayer<P>
where
    P: DataAvailabilityPayload + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        match self {
            Self::Celestia(inner) => inner.shutdown_handle(),
            Self::Noop(inner) => inner.shutdown_handle(),
        }
    }

    fn start(self) {
        match self {
            Self::Celestia(inner) => inner.start(),
            Self::Noop(inner) => inner.start(),
        }
    }
}

impl<P> DataAvailabilityBackendBuilder for AnyDataAvailabilityLayerBuilder<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Backend = AnyDataAvailabilityLayer<P>;

    fn build(self) -> Result<Self::Backend> {
        Ok(match self {
            Self::Celestia(inner) => AnyDataAvailabilityLayer::Celestia(Box::new(inner.build()?)),
            Self::Noop(inner) => AnyDataAvailabilityLayer::Noop(inner.build()?),
        })
    }

    fn last_pointer(self, last_pointer: Option<DataAvailabilityPointer>) -> Self {
        match self {
            Self::Celestia(inner) => Self::Celestia(Box::new(inner.last_pointer(last_pointer))),
            Self::Noop(inner) => Self::Noop(inner.last_pointer(last_pointer)),
        }
    }

    fn proof_channel(
        self,
        proof_channel: Receiver<<Self::Backend as DataAvailabilityBackend>::Payload>,
    ) -> Self {
        match self {
            Self::Celestia(inner) => Self::Celestia(Box::new(inner.proof_channel(proof_channel))),
            Self::Noop(inner) => Self::Noop(inner.proof_channel(proof_channel)),
        }
    }

    fn cursor_channel(
        self,
        cursor_channel: Sender<
            DataAvailabilityCursor<<Self::Backend as DataAvailabilityBackend>::Payload>,
        >,
    ) -> Self {
        match self {
            Self::Celestia(inner) => Self::Celestia(Box::new(inner.cursor_channel(cursor_channel))),
            Self::Noop(inner) => Self::Noop(inner.cursor_channel(cursor_channel)),
        }
    }
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
