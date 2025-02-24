use crate::data_availability::DataAvailabilityPointer;
use anyhow::Result;
use std::future::Future;

mod in_memory;
pub use in_memory::InMemoryStorageBackend;

mod sql_lite;
pub use sql_lite::SqliteDb;

pub trait StorageBackend {
    fn get_chain_head(&self) -> impl Future<Output = ChainHead>;

    fn set_chain_head(&mut self, block: BlockWithDa) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChainHead {
    Genesis,
    Block(BlockWithDa),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BlockWithDa {
    pub height: u64,
    pub da_pointer: DataAvailabilityPointer,
}

pub enum Step {
    Snos,
    Bridge,
}
pub enum Query {
    SnosProof,
    BridgeProof,
    BridgeTrace,
}
pub trait PersistantStorage {
    fn initialize_block(&self, block_number: u32) -> impl Future<Output = Result<()>> + Send;
    fn remove_block(&self, block_number: u32) -> impl Future<Output = Result<()>> + Send;
    fn add_pie(
        &self,
        block_number: u32,
        pie: Vec<u8>,
        step: Step,
    ) -> impl Future<Output = Result<()>> + Send;
    fn get_pie(
        &self,
        block_number: u32,
        step: Step,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn add_proof(
        &self,
        block_number: u32,
        proof: Vec<u8>,
        step: Step,
    ) -> impl Future<Output = Result<()>> + Send;
    fn get_proof(
        &self,
        block_number: u32,
        step: Step,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn add_query_id(
        &self,
        block_number: u32,
        query_id: String,
        query_type: Query,
    ) -> impl Future<Output = Result<()>> + Send;
    fn get_query_id(
        &self,
        block_number: u32,
        query_type: Query,
    ) -> impl Future<Output = Result<String>> + Send;
    fn set_status(
        &self,
        block_number: u32,
        status: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn get_status(&self, block_number: u32) -> impl Future<Output = Result<String>> + Send;
}
