// short_string.rs
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::Felt;

#[derive(Clone, PartialEq, Eq, Hash, Default, Copy)]
pub struct ShortString {
    data: [u8; 31],
    len: u8,
}

impl ShortString {
    pub const fn from_ascii(s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len();

        assert!(len <= 31, "string is too long to be a Cairo short string");

        let mut data = [0u8; 31];
        let mut i = 0;
        while i < len {
            let b = bytes[i];
            assert!(b.is_ascii(), "invalid ASCII character in string");
            data[i] = b;
            i += 1;
        }

        Self {
            data,
            len: len as u8,
        }
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.data.as_ptr(), self.len as usize) }
    }
}

impl From<ShortString> for Felt {
    fn from(string: ShortString) -> Self {
        Self::from(&string)
    }
}

impl From<&ShortString> for Felt {
    fn from(string: &ShortString) -> Self {
        Felt::from_bytes_be_slice(string.as_bytes())
    }
}

pub fn compute_starknet_os_config_hash(
    chain_id: Felt,
    deprecated_fee_token: Felt,
    fee_token: Felt,
) -> Felt {
    const STARKNET_OS_CONFIG_VERSION: ShortString = ShortString::from_ascii("StarknetOsConfig2");

    compute_hash_on_elements(&[
        STARKNET_OS_CONFIG_VERSION.into(),
        chain_id,
        deprecated_fee_token,
        fee_token,
    ])
}

#[cfg(test)]
mod test {
    use super::*;
    use starknet::core::chain_id::{MAINNET, SEPOLIA};
    use starknet::core::types::Felt;
    use starknet::macros::felt;
    const ETH_FEE_TOKEN: Felt =
        felt!("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");
    const STRK_FEE_TOKEN: Felt =
        felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
    #[test]
    fn test_short_string() {
        let s = ShortString::from_ascii("Hello, world!");
        assert_eq!(s.as_bytes(), b"Hello, world!");

        let felt: Felt = s.into();
        let expected_felt = Felt::from_bytes_be_slice(b"Hello, world!");
        assert_eq!(felt, expected_felt);
    }
    #[test]
    fn calculate_config_hash_mainnet() {
        let expected = felt!("0x5ba2078240f1585f96424c2d1ee48211da3b3f9177bf2b9880b4fc91d59e9a2");
        let chain = MAINNET;

        let computed = compute_starknet_os_config_hash(chain, ETH_FEE_TOKEN, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }

    #[test]
    fn calculate_config_hash_testnet() {
        let expected = felt!("0x504fa6e5eb930c0d8329d4a77d98391f2730dab8516600aeaf733a6123432");
        let chain = SEPOLIA;

        let computed = compute_starknet_os_config_hash(chain, ETH_FEE_TOKEN, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }
}
