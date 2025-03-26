use starknet_os::io::output::{
    deserialize_os_output, ContractChanges, OsStateDiff, StarknetOsOutput,
};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;
use swiftness::types::StarkProof;
use tokio::{fs, sync::mpsc::Receiver};

use crate::{
    prover::SnosProof,
    service::{Daemon, FinishHandle},
    utils::calculate_output,
};

use super::{Aggregator, AggregatorBuilder};

#[derive(Debug)]
pub struct AggregatorMock {
    channel: Receiver<SnosProof<StarkProof>>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct AggregatorMockBuilder {
    channel: Option<Receiver<SnosProof<StarkProof>>>,
}

impl AggregatorMockBuilder {
    pub fn new() -> Self {
        Self { channel: None }
    }
}

impl AggregatorMock {
    pub async fn run(mut self) {
        let first_block = self.channel.recv().await.unwrap();
        println!("Received 1 proof: {:?}", first_block.block_number);

        let proof_output = calculate_output(&first_block.proof);
        let mut output_iter = proof_output.iter().copied();
        output_iter.nth(2); // Skip the first 3 elements as they are bootloader related
        let mut squashing_result: StarknetOsOutput =
            deserialize_os_output(&mut output_iter).unwrap();

        while let Some(proof) = self.channel.recv().await {
            println!("Received proof: {:?}", proof.block_number);
            let proof_output = calculate_output(&proof.proof);
            let mut output_iter = proof_output.iter().copied();
            output_iter.nth(2); // Skip the first 3 elements as they are bootloader related
            let os_output: StarknetOsOutput = deserialize_os_output(&mut output_iter).unwrap();
            fs::write(
                format!("state_diff{}.json", os_output.new_block_number),
                serde_json::to_string(&os_output).unwrap(),
            )
            .await
            .unwrap(); //debug purpose

            squashing_result.final_root = os_output.final_root;
            squashing_result.new_block_number = os_output.new_block_number;
            squashing_result.new_block_hash = os_output.new_block_hash;
            squashing_result
                .messages_to_l1
                .extend(os_output.messages_to_l1);
            squashing_result
                .messages_to_l2
                .extend(os_output.messages_to_l2);

            let state_diff = os_output.state_diff.unwrap();
            let squashed_diff =
                squash_state_diff(squashing_result.state_diff.clone().unwrap(), state_diff);
            squashing_result.state_diff = Some(squashed_diff);
            fs::write(
                "state_diff.json",
                serde_json::to_string(&squashing_result).unwrap(),
            )
            .await
            .unwrap();
        }
    }
}

pub fn squash_state_diff(old: OsStateDiff, new: OsStateDiff) -> OsStateDiff {
    let result = OsStateDiff {
        classes: squash_classes(old.classes, new.classes),
        contract_changes: squash_contract_changes(old.contract_changes, new.contract_changes),
    };
    result
}
pub fn squash_contract_changes(
    mut old: Vec<ContractChanges>,
    new: Vec<ContractChanges>,
) -> Vec<ContractChanges> {
    for new_contract_change in &new {
        if let Some(existing_change) = old.iter_mut().find(|c| c.addr == new_contract_change.addr) {
            existing_change.class_hash = new_contract_change.class_hash;
            existing_change.nonce = new_contract_change.nonce;
            for (k, v) in &new_contract_change.storage_changes {
                existing_change.storage_changes.insert(*k, *v);
            }
        } else {
            // If the contract change is not present in the old list, add it
            old.push(new_contract_change.clone());
        }
    }
    old
}
pub fn squash_classes(
    mut old: HashMap<Felt, Felt>,
    new: HashMap<Felt, Felt>,
) -> HashMap<Felt, Felt> {
    for (k, v) in &new {
        old.insert(*k, *v);
    }
    old
}

#[test]
fn test_squashing() {
    let old_state_diff = OsStateDiff {
        contract_changes: vec![
            ContractChanges {
                addr: Felt::from(1),
                nonce: Felt::from(1),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(10), Felt::from(100));
                    map.insert(Felt::from(20), Felt::from(200));
                    map
                },
                class_hash: None,
            },
            ContractChanges {
                addr: Felt::from(2),
                nonce: Felt::from(1),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(30), Felt::from(300));
                    map
                },
                class_hash: None,
            },
        ],
        classes: HashMap::new(),
    };
    let new_state_diff = OsStateDiff {
        contract_changes: vec![
            ContractChanges {
                addr: Felt::from(1),
                nonce: Felt::from(2),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(10), Felt::from(150));
                    map.insert(Felt::from(25), Felt::from(250));
                    map
                },
                class_hash: None,
            },
            ContractChanges {
                addr: Felt::from(3),
                nonce: Felt::from(1),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(40), Felt::from(400));
                    map
                },
                class_hash: None,
            },
        ],
        classes: HashMap::new(),
    };
    let squashed = squash_state_diff(old_state_diff, new_state_diff);
    let expected = OsStateDiff {
        contract_changes: vec![
            ContractChanges {
                addr: Felt::from(1),
                nonce: Felt::from(2),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(10), Felt::from(150));
                    map.insert(Felt::from(20), Felt::from(200));
                    map.insert(Felt::from(25), Felt::from(250));
                    map
                },
                class_hash: None,
            },
            ContractChanges {
                addr: Felt::from(2),
                nonce: Felt::from(1),
                storage_changes: {
                    let mut map = HashMap::new();
                    map.insert(Felt::from(30), Felt::from(300));
                    map
                },
                class_hash: None,
            },
        ],
        classes: HashMap::new(),
    };
    assert_eq!(squashed, expected);
}
impl AggregatorBuilder for AggregatorMockBuilder {
    type Aggregator = AggregatorMock;

    fn build(self) -> anyhow::Result<Self::Aggregator> {
        Ok(AggregatorMock {
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("channel is required"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn channel(mut self, channel: Receiver<SnosProof<StarkProof>>) -> Self {
        self.channel = Some(channel);
        self
    }
}
impl Aggregator for AggregatorMock {}

impl Daemon for AggregatorMock {
    fn shutdown_handle(&self) -> crate::service::ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
