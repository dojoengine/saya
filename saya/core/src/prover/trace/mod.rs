use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use std::future::Future;

pub mod atlantic;
pub mod http_prover;

pub use atlantic::AtlanticTraceGenerator;
pub use http_prover::HttpProverTraceGen;

#[derive(Debug, Clone)]
pub enum TraceGenerator {
    Atlantic(AtlanticTraceGenerator),
    HttpProver(Box<HttpProverTraceGen>),
}

pub trait LayoutBridgeTraceGenerator: Send + Sync {
    fn generate_trace(
        &self,
        program: Vec<u8>,
        input: Vec<u8>,
        label: Option<String>,
    ) -> impl Future<Output = Result<CairoPie>> + Send;
}

impl LayoutBridgeTraceGenerator for TraceGenerator {
    async fn generate_trace(
        &self,
        program: Vec<u8>,
        input: Vec<u8>,
        label: Option<String>,
    ) -> Result<CairoPie> {
        match self {
            Self::Atlantic(inner) => {
                inner
                    .generate_trace(label.unwrap_or_default().as_str(), program, input)
                    .await
            }
            Self::HttpProver(inner) => inner.generate_trace(program, input).await,
        }
    }
}
