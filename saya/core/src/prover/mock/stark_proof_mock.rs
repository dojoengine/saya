use starknet::{core::types::Felt, macros::felt};
use swiftness::TransformTo;
use swiftness_air::types::AddrValue;
use swiftness_stark::types::StarkProof;

/// Builder for a [`StarkProof`] mock.
pub trait StarkProofMockBuilder {
    /// Loads the proof from the sample_proof.json file and setup the output from the given felts.
    /// The annotations have been trimmed to only contain the minimal information to be parsed and reduce the size from 800KB to 190KB.
    ///
    /// # Arguments
    ///
    /// * `output` - The output of the proof to be set.
    ///
    /// # Returns
    ///
    /// A [`StarkProof`] from the mocked content. This proof is not
    /// valid to verify, and mostly used for testing purposes.
    fn mock_from_output(output: &[Felt]) -> StarkProof;
}

impl StarkProofMockBuilder for StarkProof {
    fn mock_from_output(output: &[Felt]) -> StarkProof {
        let json = include_str!("./sample_proof.json");
        let mut proof: StarkProof = swiftness::parse(json).unwrap().transform_to();

        let output_len = output.len();
        let main_page_last_addr: Felt = felt!("0xc387");

        let output_segment = &mut proof.public_input.segments[2];

        output_segment.begin_addr = main_page_last_addr;
        output_segment.stop_ptr = main_page_last_addr + Felt::from(output_len);

        let mut addr_values = output
            .into_iter()
            .enumerate()
            .map(|(i, felt)| AddrValue {
                value: *felt,
                address: main_page_last_addr + Felt::from(i),
            })
            .collect::<Vec<AddrValue>>();

        proof.public_input.main_page.0.append(&mut addr_values);

        proof
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::calculate_output;

    #[test]
    fn test_calculate_output_from_mocked_proof() {
        let proof = StarkProof::mock_from_output(&[
            felt!("0x1"),
            felt!("0x2"),
            felt!("0x3"),
            felt!("0x4"),
            felt!("0x5"),
            felt!("0x6"),
        ]);

        let output = calculate_output(&proof);
        assert_eq!(
            output,
            vec![
                felt!("0x1"),
                felt!("0x2"),
                felt!("0x3"),
                felt!("0x4"),
                felt!("0x5"),
                felt!("0x6")
            ]
        );
    }
}
