//! TEE fact registration — verifies a TEE proof and calls the dedicated Piltover TEE entry point.

use anyhow::Result;
use log::info;
use starknet::core::types::Call;
use starknet_types_core::felt::Felt;

use crate::{
    data_availability::DataAvailabilityPointer,
    settlement::fact_registration::FactRegistrar,
};

/// Verifies a TEE proof on-chain and constructs the Piltover `update_state_tee` call.
///
/// This registrar covers the new TEE proving pipeline (see [`crate::orchestrator::TeeOrchestrator`])
/// where an enclave proof is produced by an external TEE prover service and verified against a
/// dedicated on-chain TEE verifier contract.
///
/// # TODO
/// - Retrieve the TEE proof from the DB (requires a new `Step::Tee` storage step).
/// - Call the on-chain TEE verifier contract.
/// - Fill in the correct `update_state_tee` calldata format once the contract is deployed.
#[derive(Debug)]
pub struct TeeFactRegistrar {
    piltover_address: Felt,
    // TODO: add `tee_verifier_address: Felt` once the verifier contract is deployed.
}

impl TeeFactRegistrar {
    pub fn new(piltover_address: Felt) -> Self {
        Self { piltover_address }
    }
}

impl FactRegistrar for TeeFactRegistrar {
    fn build_settlement_call(
        &self,
        block_number: u64,
        _da_pointer: Option<DataAvailabilityPointer>,
    ) -> impl std::future::Future<Output = Result<Option<Call>>> + Send + '_ {
        async move {
            // TODO: implement TEE fact registration.
            //
            // Expected flow:
            //   1. Retrieve the TEE proof from the DB (Step::Tee, once that storage step
            //      is added).
            //   2. Call the on-chain TEE verifier contract with the proof bytes and wait
            //      for confirmation.
            //   3. Construct and return the `update_state_tee` Call with the appropriate
            //      calldata for the Piltover TEE entry point.
            //
            // For now this is a no-op so the orchestrator can be exercised end-to-end
            // before the on-chain verifier contract is deployed.
            info!(block_number; "TEE fact registration not yet implemented — skipping settlement");

            Ok(None)
        }
    }
}
