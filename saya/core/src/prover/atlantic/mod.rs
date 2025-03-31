use anyhow::Result;
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;

mod client;

mod snos;
pub use snos::{AtlanticSnosProver, AtlanticSnosProverBuilder};

mod shared;

mod layout_bridge;
pub use client::AtlanticClient;
pub use layout_bridge::{AtlanticLayoutBridgeProver, AtlanticLayoutBridgeProverBuilder};
pub use snos::compress_pie;

pub trait AtlanticProof: Sized {
    fn parse(raw_proof: String) -> Result<Self>;
    fn from_stark_proof(stark_proof: StarkProof) -> Self;
}

impl AtlanticProof for StarkProof {
    fn from_stark_proof(stark_proof: StarkProof) -> Self {
        stark_proof
    }
    fn parse(raw_proof: String) -> Result<Self> {
        Ok(swiftness::parse(raw_proof)?.transform_to())
    }
}

impl AtlanticProof for String {
    fn parse(raw_proof: String) -> Result<Self> {
        Ok(raw_proof)
    }
    fn from_stark_proof(stark_proof: StarkProof) -> Self {
        serde_json::to_string(&stark_proof).unwrap()
    }
}
