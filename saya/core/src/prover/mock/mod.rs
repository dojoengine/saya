mod layout_bridge;
pub use layout_bridge::{LayoutBridgeMockProver, LayoutBridgeMockProverBuilder};

mod stark_proof_mock;
pub use stark_proof_mock::StarkProofMockBuilder;

const PROOF_GENERATION_JOB_NAME: &str = "PROOF_GENERATION_MOCK";
