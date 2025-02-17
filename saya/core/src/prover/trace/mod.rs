use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use std::future::Future;

pub mod atlantic;
pub mod http_prover;

pub use atlantic::AtlanticTraceGenerator;
pub use http_prover::HttpProverTraceGen;

#[derive(Debug)]
pub enum TraceGenerator {
    Atlantic(AtlanticTraceGenerator),
    HttpProver(Box<HttpProverTraceGen>),
}

pub trait LayoutBridgeTraceGenerator: Send + Sync {
    fn generate_trace(
        &self,
        program: Vec<u8>,
        input: Vec<u8>,
    ) -> impl Future<Output = Result<CairoPie>> + Send;
}

impl LayoutBridgeTraceGenerator for TraceGenerator {
    async fn generate_trace(&self, program: Vec<u8>, input: Vec<u8>) -> Result<CairoPie> {
        match self {
            Self::Atlantic(inner) => inner.generate_trace(program, input).await,
            Self::HttpProver(inner) => inner.generate_trace(program, input).await,
        }
    }
}
