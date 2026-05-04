//! Helpers for synthesizing fake `TEEInput.sp1_proof` payloads when running
//! `saya-tee --mock-prove`.
//!
//! In mock mode the prover does **not** call AMD KDS, validate any cert chain,
//! or submit anything to the SP1 prover network. Instead it computes the
//! Poseidon commitment that Piltover's `validate_input` would otherwise extract
//! from a real attestation report and packages it into a Cairo-Serde-serialized
//! [`amd_tee_registry::tee_types::VerifierJournal`] which the paired
//! `piltover_mock_amd_tee_registry` Cairo contract trivially round-trips.
//!
//! The serialized journal is stored verbatim in [`TeeProof::data`] and is
//! consumed unmodified by [`crate::settlement::build_tee_calldata`] when its
//! `mock_prove` branch is taken — no `OnchainProof::decode_json` /
//! `StarknetCalldata::from_proof` round-trip is performed.
//!
//! ## Wire format of `TEEInput.sp1_proof` in mock mode
//!
//! A Cairo `Span<felt252>` matching the Cairo Serde of:
//!
//! ```cairo
//! VerifierJournal {
//!     result: VerificationResult::Success,   // 1 felt (variant 0)
//!     timestamp: 0,                          // 1 felt (u64)
//!     processor_model: 0,                    // 1 felt (u8, Milan)
//!     raw_report: Span<u32> { len = 296, .. }, // 1 + 296 felts
//!     certs: Array<u256> { len = 0 },        // 1 felt
//!     cert_serials: Array<felt252> { len = 0 }, // 1 felt
//!     trusted_certs_prefix_len: 0,           // 1 felt (u8)
//!     storage_commitment: 0,                 // 1 felt (felt252)
//!     fork_block_number: 0,                  // 1 felt (u64)
//!     end_block_number: 0,                   // 1 felt (u64)
//! }
//! ```
//!
//! Within `raw_report`, only the 16 u32 words at the `report_data` offset
//! (u32 index 20) carry meaningful data; all other words are zero.
//!
//! ## `report_data` byte layout (v1)
//!
//! Piltover (`src/input/component.cairo`, `validate_input` for `TeeInput`)
//! decodes both halves of the 64-byte `report_data`:
//!
//! - bytes 0..32  → first-half v1 commitment
//! - bytes 32..64 → `katana_tee_config_hash`
//!
//! and asserts:
//!
//! 1. `tee_input.katana_tee_config_hash == piltover.config_hash`
//! 2. second-half decoded felt `== tee_input.katana_tee_config_hash`
//! 3. first-half decoded felt `== Poseidon([
//!      'KatanaTeeReport1', 'KatanaTeeAppchain',
//!      prev_state_root, state_root, prev_block_hash, block_hash,
//!      prev_block_number, block_number,
//!      messages_commitment, katana_tee_config_hash,
//!    ])`
//!
//! Each 32-byte half is packed into 8 u32 words by reading each 4-byte BE
//! chunk as a little-endian u32, so that Piltover's
//! `u128_byte_reverse(get_u128_at(...))` reconstruction yields the original
//! BE felt. See [`felt_to_report_words`].

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

/// Number of u32 words in an AMD SEV-SNP attestation report (1184 bytes / 4).
pub const ATTESTATION_REPORT_WORDS: usize = 296;

/// u32 word offset of the `report_data` field within an attestation report
/// (byte offset 0x50 / 4).
pub const REPORT_DATA_WORD_OFFSET: usize = 20;

/// Short-string `'KatanaTeeReport1'` — version tag for the v1 report-data schema.
pub const KATANA_TEE_REPORT_VERSION: Felt =
    Felt::from_hex_unchecked("0x4b6174616e615465655265706f727431");

/// Short-string `'KatanaTeeAppchain'` — mode tag for appchain settlement.
pub const KATANA_TEE_APPCHAIN_MODE: Felt =
    Felt::from_hex_unchecked("0x4b6174616e61546565417070636861696e");

/// Computes the v1 Poseidon commitment Piltover asserts against the first half
/// of `report_data` for the appchain TEE settlement path.
///
/// Mirrors the inline recomputation in
/// `cartridge-gg/piltover` `src/input/component.cairo:198-207` and Katana's
/// `compute_report_data_appchain`.
pub fn compute_appchain_commitment(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    messages_commitment: Felt,
    katana_tee_config_hash: Felt,
) -> Felt {
    Poseidon::hash_array(&[
        KATANA_TEE_REPORT_VERSION,
        KATANA_TEE_APPCHAIN_MODE,
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        messages_commitment,
        katana_tee_config_hash,
    ])
}

/// Packs a felt into 8 u32 words such that Piltover's `u128_byte_reverse(
/// get_u128_at(..))` reconstruction yields back the original felt.
///
/// Each 4-byte BE chunk of `value.to_bytes_be()` is read as a little-endian u32.
fn felt_to_report_words(value: Felt) -> [Felt; 8] {
    let bytes = value.to_bytes_be();
    let mut words = [Felt::ZERO; 8];
    for i in 0..8 {
        let chunk = [
            bytes[i * 4],
            bytes[i * 4 + 1],
            bytes[i * 4 + 2],
            bytes[i * 4 + 3],
        ];
        let word = u32::from_le_bytes(chunk);
        words[i] = Felt::from(word);
    }
    words
}

/// Builds a 296-word `raw_report` whose `report_data` carries the v1
/// commitment in the first 32 bytes and `katana_tee_config_hash` in the
/// second 32 bytes. All other words are zero.
pub fn build_raw_report(commitment: Felt, katana_tee_config_hash: Felt) -> Vec<Felt> {
    let mut raw_report = vec![Felt::ZERO; ATTESTATION_REPORT_WORDS];
    let first_half = felt_to_report_words(commitment);
    let second_half = felt_to_report_words(katana_tee_config_hash);
    raw_report[REPORT_DATA_WORD_OFFSET..REPORT_DATA_WORD_OFFSET + 8].copy_from_slice(&first_half);
    raw_report[REPORT_DATA_WORD_OFFSET + 8..REPORT_DATA_WORD_OFFSET + 16]
        .copy_from_slice(&second_half);
    raw_report
}

/// Cairo-Serde-serializes a stub `VerifierJournal` whose `raw_report` field
/// encodes the v1 commitment + config hash in the positions Piltover reads.
///
/// The output is a `Vec<Felt>` matching what
/// `Serde::<VerifierJournal>::deserialize` reconstructs in
/// `piltover_mock_amd_tee_registry::verify_sp1_proof`.
pub fn serialize_mock_journal(commitment: Felt, katana_tee_config_hash: Felt) -> Vec<Felt> {
    let raw_report = build_raw_report(commitment, katana_tee_config_hash);

    // 1 (result) + 1 (timestamp) + 1 (processor_model)
    // + 1 (raw_report len) + 296 (raw_report elements)
    // + 1 (certs len) + 1 (cert_serials len) + 1 (trusted_certs_prefix_len)
    // + 1 (storage_commitment) + 1 (fork_block_number) + 1 (end_block_number)
    // = 306 felts.
    let mut felts = Vec::with_capacity(306);

    // result: VerificationResult::Success → variant index 0
    felts.push(Felt::ZERO);
    // timestamp: u64
    felts.push(Felt::ZERO);
    // processor_model: u8 (Milan == 0)
    felts.push(Felt::ZERO);
    // raw_report: Span<u32> = [length, ...elements]
    felts.push(Felt::from(raw_report.len() as u64));
    felts.extend(raw_report);
    // certs: Array<u256> = [length, ...]
    felts.push(Felt::ZERO);
    // cert_serials: Array<felt252> = [length, ...]
    felts.push(Felt::ZERO);
    // trusted_certs_prefix_len: u8
    felts.push(Felt::ZERO);
    // storage_commitment: felt252
    felts.push(Felt::ZERO);
    // fork_block_number: u64
    felts.push(Felt::ZERO);
    // end_block_number: u64
    felts.push(Felt::ZERO);

    felts
}

/// Encodes a `Vec<Felt>` as raw big-endian bytes for storage in
/// [`crate::TeeProof::data`]. Each felt occupies exactly 32 bytes.
pub fn felts_to_bytes(felts: &[Felt]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(felts.len() * 32);
    for felt in felts {
        bytes.extend_from_slice(&felt.to_bytes_be());
    }
    bytes
}

/// Decodes a `Vec<Felt>` from raw big-endian bytes produced by
/// [`felts_to_bytes`].
///
/// Returns `None` if the byte slice length is not a multiple of 32.
pub fn bytes_to_felts(bytes: &[u8]) -> Option<Vec<Felt>> {
    if !bytes.len().is_multiple_of(32) {
        return None;
    }
    let mut felts = Vec::with_capacity(bytes.len() / 32);
    for chunk in bytes.chunks_exact(32) {
        let mut buf = [0u8; 32];
        buf.copy_from_slice(chunk);
        felts.push(Felt::from_bytes_be(&buf));
    }
    Some(felts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn felt_round_trip_via_bytes() {
        let felts = vec![
            Felt::ZERO,
            Felt::from(1u64),
            Felt::from(0xdeadbeef_u64),
            Felt::from_hex("0x123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap(),
        ];
        let bytes = felts_to_bytes(&felts);
        let decoded = bytes_to_felts(&bytes).unwrap();
        assert_eq!(decoded, felts);
    }

    #[test]
    fn raw_report_has_canonical_size() {
        let raw_report = build_raw_report(Felt::from(42u64), Felt::from(7u64));
        assert_eq!(raw_report.len(), ATTESTATION_REPORT_WORDS);
    }

    #[test]
    fn report_data_zero_outside_64_bytes() {
        let raw_report = build_raw_report(Felt::from(42u64), Felt::from(7u64));
        // Words [0..20) and [36..296) must be zero; [20..36) carry the v1
        // commitment + config_hash halves.
        for (i, word) in raw_report.iter().enumerate().take(20) {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
        for (i, word) in raw_report
            .iter()
            .enumerate()
            .take(ATTESTATION_REPORT_WORDS)
            .skip(36)
        {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
    }

    #[test]
    fn report_data_round_trips_both_halves() {
        // Mirror Piltover's reconstruction for both halves:
        //   limb_i = sum(w_{4i+j} * 2^(32*j)) for j in 0..4   (little-endian u32 → u128)
        //   commitment   = (u128_byte_reverse(limb0) << 128) | u128_byte_reverse(limb1)
        //   config_hash  = (u128_byte_reverse(limb2) << 128) | u128_byte_reverse(limb3)
        let commitment =
            Felt::from_hex("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let config_hash =
            Felt::from_hex("0x00c53b8a360950659fdafc5f9e42ab39db23d3ac909bafe9f9428fd72e57828")
                .unwrap();
        let raw_report = build_raw_report(commitment, config_hash);

        let read_limb = |start: usize| -> u128 {
            let w0 = u128::from(
                raw_report[start].to_bytes_be()[31] as u32
                    | (raw_report[start].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start].to_bytes_be()[28] as u32) << 24,
            );
            let w1 = u128::from(
                raw_report[start + 1].to_bytes_be()[31] as u32
                    | (raw_report[start + 1].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 1].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 1].to_bytes_be()[28] as u32) << 24,
            );
            let w2 = u128::from(
                raw_report[start + 2].to_bytes_be()[31] as u32
                    | (raw_report[start + 2].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 2].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 2].to_bytes_be()[28] as u32) << 24,
            );
            let w3 = u128::from(
                raw_report[start + 3].to_bytes_be()[31] as u32
                    | (raw_report[start + 3].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 3].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 3].to_bytes_be()[28] as u32) << 24,
            );
            w0 + (w1 << 32) + (w2 << 64) + (w3 << 96)
        };

        let limbs = [
            read_limb(REPORT_DATA_WORD_OFFSET),
            read_limb(REPORT_DATA_WORD_OFFSET + 4),
            read_limb(REPORT_DATA_WORD_OFFSET + 8),
            read_limb(REPORT_DATA_WORD_OFFSET + 12),
        ];

        let reconstruct = |hi: u128, lo: u128| -> Felt {
            let mut bytes = [0u8; 32];
            bytes[..16].copy_from_slice(&hi.swap_bytes().to_be_bytes());
            bytes[16..].copy_from_slice(&lo.swap_bytes().to_be_bytes());
            Felt::from_bytes_be(&bytes)
        };

        assert_eq!(reconstruct(limbs[0], limbs[1]), commitment);
        assert_eq!(reconstruct(limbs[2], limbs[3]), config_hash);
    }

    #[test]
    fn serialized_journal_has_expected_length() {
        let felts = serialize_mock_journal(Felt::from(1u64), Felt::from(2u64));
        // Expected total = 306 felts (see docstring).
        assert_eq!(felts.len(), 306);
    }

    #[test]
    fn version_tags_decode_to_expected_strings() {
        assert_eq!(
            &KATANA_TEE_REPORT_VERSION.to_bytes_be()[16..],
            b"KatanaTeeReport1"
        );
        assert_eq!(
            &KATANA_TEE_APPCHAIN_MODE.to_bytes_be()[15..],
            b"KatanaTeeAppchain"
        );
    }
}
