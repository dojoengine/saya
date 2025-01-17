use std::future::Future;

use crate::data_availability::DataAvailabilityPointer;

mod in_memory;
pub use in_memory::InMemoryStorageBackend;

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
