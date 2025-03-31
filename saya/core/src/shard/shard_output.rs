use cainome_cairo_serde_derive::CairoSerde;
use cairo_vm::Felt252;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, CairoSerde)]
pub struct ContractChanges {
    /// The address of the contract.
    pub addr: Felt252,
    /// The new nonce of the contract (for account contracts).
    pub nonce: Felt252,
    /// The new class hash (if changed).
    pub class_hash: Option<Felt252>,
    /// A map from storage key to its new value.
    pub storage_changes: Vec<(Felt252, Felt252)>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, CairoSerde)]
pub struct ShardOutput {
    pub state_diff: Vec<ContractChanges>,
}
