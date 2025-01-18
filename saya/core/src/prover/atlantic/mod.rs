use anyhow::Result;
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;

mod client;

mod snos;
pub use snos::{AtlanticSnosProver, AtlanticSnosProverBuilder};

mod layout_bridge;
pub use layout_bridge::{AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder};

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
