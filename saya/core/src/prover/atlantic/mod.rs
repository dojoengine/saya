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

/// Calculate the job size based on the number of steps in the pie.
/// Refer to the [Atlantic Prover](https://docs.herodotus.cloud/atlantic/sending-query) documentation for more details.
/// Larger sizes can be used for small pies, but this increases the cost.
/// The sizes affect the resources allocated to the job.
/// Available sizes are XS, S, M, and L. Size XS is purely virtual for Atlantic optimization and is interpreted as size S by SHARP.
/// While XS affects resource usage on the Atlantic backend, it has no impact on SHARP, and XS and S have the same cost in SHARP.
pub fn calculate_job_size(pie: CairoPie) -> AtlanticJobSize {
    match pie.execution_resources.n_steps {
        0..=6_499_999 => AtlanticJobSize::XS,
        6_500_000..=12_999_999 => AtlanticJobSize::S,
        13_000_000..=29_999_999 => AtlanticJobSize::M,
        _ => AtlanticJobSize::L,
    }
}
