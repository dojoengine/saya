//! No-op fact registration — skips on-chain verification, used for testing and development.
//!
//! The bridge proof is still fetched and parsed (to extract `program_output`) but no
//! transactions are sent to an integrity verifier contract.

use anyhow::Result;
use log::{debug, info};
use starknet::core::types::Call;
use starknet_types_core::felt::Felt;
use swiftness::types::StarkProof;

use crate::{
    data_availability::DataAvailabilityPointer,
    settlement::fact_registration::{build_update_state_call, FactRegistrar},
    storage::{BlockStatus, PersistantStorage, Step},
    utils::{calculate_output, extract_messages_from_program_output},
};

/// Skips on-chain proof verification and returns the `update_state` call directly.
///
/// Fetches the bridge proof from the DB, extracts `program_output`, and constructs the Piltover
/// `update_state` call — without sending any verification transactions.
///
/// Intended for development and testing workflows where the STARK pipeline is active but
/// on-chain integrity verification is not required.
#[derive(Debug)]
pub struct NoopFactRegistrar<DB> {
    piltover_address: Felt,
    db: DB,
}

impl<DB> NoopFactRegistrar<DB> {
    pub fn new(piltover_address: Felt, db: DB) -> Self {
        Self {
            piltover_address,
            db,
        }
    }
}

impl<DB> FactRegistrar for NoopFactRegistrar<DB>
where
    DB: PersistantStorage + Send + Sync + 'static,
{
    fn build_settlement_call(
        &self,
        block_number: u64,
        da_pointer: Option<DataAvailabilityPointer>,
    ) -> impl std::future::Future<Output = Result<Option<Call>>> + Send + '_ {
        async move {
            let block_number_u32: u32 = block_number.try_into()?;

            let proof_bytes = match self.db.get_proof(block_number_u32, Step::Bridge).await {
                Ok(b) => b,
                Err(e) => {
                    debug!(block_number; "No bridge proof found, skipping: {}", e);
                    return Ok(None);
                }
            };
            let raw_proof = String::from_utf8(proof_bytes)?;

            let status = self.db.get_status(block_number_u32).await?;
            match status {
                BlockStatus::BridgeProofGenerated | BlockStatus::VerifiedProof => {}
                _ => {
                    debug!(block_number; "Block in unexpected status {:?}, skipping", status);
                    return Ok(None);
                }
            }

            let layout_bridge_proof = serde_json::from_str::<StarkProof>(&raw_proof)?;
            let program_output = calculate_output(&layout_bridge_proof);

            let (messages_to_l1, messages_to_l2) =
                extract_messages_from_program_output(&mut program_output.clone().into_iter());
            for message in messages_to_l1 {
                debug!("Message to L1: {:?}", message);
            }
            for message in messages_to_l2 {
                debug!("Message to L2: {:?}", message);
            }

            info!(block_number; "On-chain fact registration skipped");

            Ok(Some(build_update_state_call(
                self.piltover_address,
                program_output,
                da_pointer,
            )))
        }
    }
}
