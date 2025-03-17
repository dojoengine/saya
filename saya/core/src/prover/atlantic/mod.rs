use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use client::AtlanticJobSize;
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;

mod client;

mod snos;
pub use snos::{AtlanticSnosProver, AtlanticSnosProverBuilder};

mod layout_bridge;
pub use client::AtlanticClient;
pub use layout_bridge::{AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder};
pub use snos::compress_pie;

pub trait AtlanticProof: Sized {
    fn parse(raw_proof: String) -> Result<Self>;
}

impl AtlanticProof for StarkProof {
    fn parse(raw_proof: String) -> Result<Self> {
        Ok(swiftness::parse(raw_proof)?.transform_to())
    }
}

impl AtlanticProof for String {
    fn parse(raw_proof: String) -> Result<Self> {
        Ok(raw_proof)
    }
}

pub fn calculate_job_size(pie: CairoPie) -> AtlanticJobSize {
    match pie.execution_resources.n_steps {
        0..=12_999_999 => AtlanticJobSize::S,
        13_000_000..=29_999_999 => AtlanticJobSize::M,
        _ => AtlanticJobSize::L,
    }
}
