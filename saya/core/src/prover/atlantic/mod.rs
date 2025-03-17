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
/// Check the [Atlantic Prover](https://docs.herodotus.cloud/atlantic/sending-query) documentation for more information.
/// We can use bigger sizes for small pies as well, but this increases the cost.
/// The sizes have impact on amount of recoursces dedicated to the job.
/// The sizes are XS, S, M, L. We skip XS as the limit for number of steps is not know at the moment (17.03.2025).
pub fn calculate_job_size(pie: CairoPie) -> AtlanticJobSize {
    match pie.execution_resources.n_steps {
        0..=6_499_999 => AtlanticJobSize::XS,
        6_500_000..=12_999_999 => AtlanticJobSize::S,
        13_000_000..=29_999_999 => AtlanticJobSize::M,
        _ => AtlanticJobSize::L,
    }
}
