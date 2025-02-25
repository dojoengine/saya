use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use std::future::Future;

pub mod atlantic;
pub mod http_prover;

pub use atlantic::AtlanticTraceGenerator;
pub use http_prover::HttpProverTraceGen;

use crate::storage::PersistantStorage;

#[derive(Debug, Clone)]
pub enum TraceGenerator {
    Atlantic(AtlanticTraceGenerator),
    HttpProver(Box<HttpProverTraceGen>),
}

pub trait LayoutBridgeTraceGenerator<DB>: Send + Sync
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn generate_trace(
        &self,
        program: Vec<u8>,
        block_number: u32,
        input: Vec<u8>,
        db: DB,
    ) -> impl Future<Output = Result<CairoPie>> + Send;
}

impl<DB> LayoutBridgeTraceGenerator<DB> for TraceGenerator
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn generate_trace(
        &self,
        program: Vec<u8>,
        block_number: u32,
        input: Vec<u8>,
        db: DB,
    ) -> Result<CairoPie> {
        match self {
            Self::Atlantic(inner) => inner.generate_trace(program, block_number, input, db).await,
            Self::HttpProver(inner) => inner.generate_trace(program, block_number, input, db).await,
        }
    }
}
