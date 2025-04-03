use cainome_cairo_serde::CairoSerde;
use starknet::core::codec::Encode;
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::types::Call,
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::LocalWallet,
};
use starknet_os::io::output::{
    deserialize_os_output, ContractChanges, OsStateDiff, StarknetOsOutput,
};
use starknet_types_core::felt::Felt;
use std::{collections::HashMap, sync::Arc};
use swiftness::types::StarkProof;
use tokio::sync::mpsc::Receiver;

use crate::shard::shard_output::{CRDType, ContractChanges as ShardContractChanges, ShardOutput, StorageChange};
use crate::{
    prover::SnosProof,
    service::{Daemon, FinishHandle},
    utils::calculate_output,
};

use super::{Aggregator, AggregatorBuilder};

#[derive(Debug)]
pub struct AggregatorMock {
    shard_id: u32,
    proxy_contract_address: Felt,
    channel: Receiver<SnosProof<StarkProof>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct AggregatorMockBuilder {
    shard_id: u32,
    proxy_contract_address: Felt,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    channel: Option<Receiver<SnosProof<StarkProof>>>,
}

impl AggregatorMockBuilder {
    pub fn new(
        shard_id: u32,
        account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
        proxy_contract_address: Felt,
    ) -> Self {
        Self {
            channel: None,
            shard_id,
            account,
            proxy_contract_address,
        }
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
            let state_diff = os_output.state_diff.unwrap();

            let squashed_diff =
                squash_state_diff(squashing_result.state_diff.clone().unwrap(), state_diff);
            squashing_result.state_diff = Some(squashed_diff);
        }

        let mut shard_output = ShardOutput { state_diff: vec![] };

        for contract_change in squashing_result.state_diff.unwrap().contract_changes {
            shard_output.state_diff.push(ShardContractChanges {
                addr: contract_change.addr,
                nonce: contract_change.nonce,
                class_hash: contract_change.class_hash,
                storage_changes: contract_change
                    .storage_changes
                    .iter()
                    .map(|(k, v)| StorageChange {
                        key: *k,
                        value: *v,
                        crd_type: CRDType::Add,
                    })
                    .collect(),
            });
        }

        let calldata = ShardOutput::cairo_serialize(&shard_output);
        println!("Finished squashing proofs");
        send_transaction(
            self.proxy_contract_address,
            calldata,
            self.account,
            self.shard_id.into(),
        )
        .await;
        self.finish_handle.finish();
    }
}

pub fn squash_state_diff(old: OsStateDiff, new: OsStateDiff) -> OsStateDiff {
    OsStateDiff {
        classes: squash_classes(old.classes, new.classes),
        contract_changes: squash_contract_changes(old.contract_changes, new.contract_changes),
    }
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

#[derive(Debug, Encode)]
struct UpdateStateCalldata {
    snos_output: Vec<Felt>,
    shard_id: Felt,
}

pub async fn send_transaction(
    contract_address: Felt,
    snos_output: Vec<Felt>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    shard_id: Felt,
) {
    let selector = selector!("update_contract_state");
    let call = Call {
        to: contract_address,
        selector,
        calldata: {
            let calldata = UpdateStateCalldata {
                snos_output,
                shard_id,
            };

            let mut raw_calldata = vec![];
            calldata.encode(&mut raw_calldata).unwrap();
            raw_calldata
        },
    };
    let tx = account
        .execute_v3(vec![call])
        .send()
        .await
        .unwrap()
        .transaction_hash;
    println!("{}", tx);
}

impl AggregatorBuilder for AggregatorMockBuilder {
    type Aggregator = AggregatorMock;

    fn build(self) -> anyhow::Result<Self::Aggregator> {
        Ok(AggregatorMock {
            proxy_contract_address: self.proxy_contract_address,
            account: self.account,
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("channel is required"))?,
            shard_id: self.shard_id,
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
