//! Fact registration strategies for the Piltover settlement backend.
//!
//! The [`FactRegistrar`] trait decouples proof verification from the core settlement logic.
//! The settlement backend only executes the returned [`Call`]; the registrar decides how to
//! verify and what to call.
//!
//! # Implementations
//!
//! | Type | Verification | Entry point |
//! |---|---|---|
//! | [`IntegrityFactRegistrar`] | On-chain STARK integrity verifier | `update_state` |
//! | [`NoopFactRegistrar`] | None (dev/test mode) | `update_state` |
//! | [`TeeFactRegistrar`] | On-chain TEE verifier (TODO) | `update_state_tee` |

mod integrity;
mod noop;
mod tee;

pub use integrity::IntegrityFactRegistrar;
pub use noop::NoopFactRegistrar;
pub use tee::TeeFactRegistrar;

use std::future::Future;

use anyhow::Result;
use starknet::core::types::Call;
use starknet_types_core::felt::Felt;

use crate::data_availability::DataAvailabilityPointer;

// Re-exported so binary crates that depend on a different version of `starknet` can still
// reference the exact `Call` type used in the `FactRegistrar` trait.
pub use starknet::core::types::Call as SettlementCall;

/// Verifies a proof for `block_number` and constructs the settlement [`Call`].
///
/// Returns `Ok(Some(call))` when settlement should proceed, `Ok(None)` when the block should
/// be silently skipped (e.g. unexpected status), and `Err(_)` on hard failures.
///
/// Implementors are responsible for their own idempotency: if verification was already
/// submitted in a previous run they should detect that (e.g. via block status in the DB) and
/// proceed directly to building the call without re-submitting.
pub trait FactRegistrar: Send + Sync {
    fn build_settlement_call(
        &self,
        block_number: u64,
        da_pointer: Option<DataAvailabilityPointer>,
    ) -> impl Future<Output = Result<Option<Call>>> + Send + '_;
}

/// Builds the `update_state` [`Call`] for the Piltover contract.
///
/// Used by both [`IntegrityFactRegistrar`] and [`NoopFactRegistrar`] which share the same
/// Piltover entry point.
pub(super) fn build_update_state_call(
    piltover_address: Felt,
    program_output: Vec<Felt>,
    da_pointer: Option<DataAvailabilityPointer>,
) -> Call {
    use piltover::{DaLayerInfo, PiltoverInput};
    use starknet::macros::selector;

    let calldata = if let Some(pointer) = da_pointer {
        let da_layer_info = DaLayerInfo {
            height: pointer.height.into(),
            commitment: Felt::from_bytes_be(&pointer.commitment),
            namespace: pointer.namespace,
        };
        let input = PiltoverInput::LayoutBridgeOutputWithDa((program_output, da_layer_info));
        <PiltoverInput as cainome::cairo_serde::CairoSerde>::cairo_serialize(&input)
    } else {
        let input = PiltoverInput::LayoutBridgeOutputNoDa(program_output);
        <PiltoverInput as cainome::cairo_serde::CairoSerde>::cairo_serialize(&input)
    };

    Call {
        to: piltover_address,
        selector: selector!("update_state"),
        calldata,
    }
}
