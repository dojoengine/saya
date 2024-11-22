use serde::Serialize;
use serde_felt::to_felts;
use starknet::accounts::{Account, ConnectedAccount};
use starknet::core::types::{BlockId, BlockTag, Call, FunctionCall};
use starknet::core::utils::get_selector_from_name;
use starknet::providers::Provider;
use starknet_crypto::poseidon_hash_many;
use starknet_types_core::felt::Felt;
use swiftness_proof_parser::{parse, StarkProof};
use tracing::{info, trace};

use crate::errors::Error;
use crate::retry;
use crate::starknet::account::SayaStarknetAccount;

#[derive(Debug, Serialize)]
pub struct PiltoverCalldata {
    pub program_snos_output: Vec<Felt>,
    pub program_output: Vec<Felt>,
    pub onchain_data_hash: Felt,
    pub onchain_data_size: (Felt, Felt), // U256
}
#[derive(Debug)]
pub struct Piltover {
    pub contract: Felt,
    pub account: SayaStarknetAccount,
}
pub struct PiltoverState {
    pub state_root: Felt,
    pub block_number: u32,
    pub block_hash: Felt,
}

impl Piltover {
    pub async fn update_state(&self, pie_proof: String, bridge_proof: String,block_number:u32) -> Result<(), Error> {
        let parsed_proof = parse(pie_proof)?;
        let program_snos_output = calculate_output(parsed_proof);
        let parsed_proof = parse(bridge_proof)?;
        let program_output = calculate_output(parsed_proof);
        let output_hash = poseidon_hash_many(&program_output);
        let snos_output_hash = poseidon_hash_many(&program_snos_output);
        trace!("layout bridge output_hash {:?}", output_hash);
        trace!("snos pie output_hash {:?}", snos_output_hash);

        let piltover_calldata = PiltoverCalldata {
            program_snos_output,
            program_output,
            onchain_data_hash: Felt::ZERO,
            onchain_data_size: (Felt::ZERO, Felt::ZERO),
        };
        let nonce = self.account.get_nonce().await?;
        let calldata = to_felts(&piltover_calldata)?;
        let _tx = retry!(
            self.account
                .execute_v1(vec![Call {
                    to: self.contract,
                    selector: get_selector_from_name("update_state").expect("invalid selector"),
                    calldata: calldata.clone()
                }])
                .nonce(nonce)
                .send()
        )?;

        info!("Block {} settled on piltover contract {:#x}",block_number, self.contract);
        Ok(())
    }

    pub async fn get_state(&self) -> PiltoverState {
        let function_call = FunctionCall {
            contract_address: self.contract,
            entry_point_selector: get_selector_from_name("get_state").unwrap(),
            calldata: vec![],
        };

        let transaction = self
            .account
            .provider()
            .call(function_call, BlockId::Tag(BlockTag::Latest))
            .await
            .unwrap();
        let state = transaction[0];
        let block_number = transaction[1];
        let block_hash = transaction[2];
        PiltoverState {
            state_root: state,
            block_number: block_number.to_string().parse().unwrap(),
            block_hash,
        }
    }
}

pub fn calculate_output(proof: StarkProof) -> Vec<Felt> {
    let output_segment = proof.public_input.segments[2].clone();
    let output_len = output_segment.stop_ptr - output_segment.begin_addr;
    let start = proof.public_input.main_page.len() - output_len as usize;
    let end = proof.public_input.main_page.len();
    let program_output = proof.public_input.main_page[start..end]
        .iter()
        .map(|cell| cell.value.clone())
        .collect::<Vec<_>>();
    let mut felts = vec![];
    for elem in &program_output {
        felts.push(Felt::from_dec_str(&elem.to_string()).unwrap());
    }
    felts
}
