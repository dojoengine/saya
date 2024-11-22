use crate::errors::Error;

pub mod sql_lite;
pub mod tests;
pub mod utils;
#[allow(async_fn_in_trait)]
pub trait SayaProvingDb {
    async fn insert_block(
        &self,
        block_id: u32,
        query_id: &str,
        status: BlockStatus,
    ) -> Result<(), Error>;

    async fn check_status(&self, block: u32) -> Result<Block, Error>;

    async fn update_block_status(&self, block_id: u32, status: BlockStatus) -> Result<(), Error>;
    async fn list_blocks_with_status(&self, status: BlockStatus) -> Result<Vec<Block>, Error>;

    async fn update_block_query_id_for_bridge_proof(
        &self,
        block_id: u32,
        query_id: &str,
    ) -> Result<(), Error>;

    async fn insert_pie_proof(&self, block_id: u32, proof: &str) -> Result<(), Error>;
    async fn insert_bridge_proof(&self, block_id: u32, proof: &str) -> Result<(), Error>;
    async fn get_pie_proof(&self, block_id: u32) -> Result<String, Error>;
    async fn get_bridge_proof(&self, block_id: u32) -> Result<String, Error>;
    async fn list_proof(&self) -> Result<Vec<String>, Error>;
}
#[derive(Debug, Clone)]
pub struct Block {
    pub id: u32,
    pub query_id_step1: String,
    pub query_id_step2: String,
    pub status: BlockStatus,
}
#[derive(Debug, Clone, PartialEq)]
pub enum BlockStatus {
    PieSubmitted,
    Failed,
    PieProofGenerated,
    BridgeProofSubmited,
    Completed,
}
#[derive(Debug, Clone, PartialEq)]
pub enum ProverStatus {
    Proving,
    Proved,
    Failed,
}

impl BlockStatus {
    pub fn as_str(&self) -> &str {
        match self {
            BlockStatus::PieSubmitted => "PIE_SUBMITTED",
            BlockStatus::Failed => "FAILED",
            BlockStatus::PieProofGenerated => "PIE_PROOF_GENERATED",
            BlockStatus::BridgeProofSubmited => "BRIDGE_PROOF_SUBMITED",
            BlockStatus::Completed => "COMPLETED",
        }
    }
}
impl TryFrom<&str> for BlockStatus {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "PIE_SUBMITTED" => Ok(BlockStatus::PieSubmitted),
            "FAILED" => Ok(BlockStatus::Failed),
            "PIE_PROOF_GENERATED" => Ok(BlockStatus::PieProofGenerated),
            "BRIDGE_PROOF_SUBMITED" => Ok(BlockStatus::BridgeProofSubmited),
            "COMPLETED" => Ok(BlockStatus::Completed),
            _ => Err(Error::TryFromStrError("AtlanticStatus conversion error".to_string())),
        }
    }
}
