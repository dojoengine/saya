// short_string.rs
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::Felt;
use starknet::macros::short_string;

pub fn compute_starknet_os_config_hash(chain_id: Felt, fee_token: Felt) -> Felt {
    const STARKNET_OS_CONFIG_VERSION: Felt = short_string!("StarknetOsConfig3");

    compute_hash_on_elements(&[STARKNET_OS_CONFIG_VERSION.into(), chain_id, fee_token])
}

#[cfg(test)]
mod test {
    use super::*;
    use starknet::core::chain_id::{MAINNET, SEPOLIA};
    use starknet::core::types::Felt;
    use starknet::macros::felt;
    const STRK_FEE_TOKEN: Felt =
        felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

    #[test]
    fn calculate_config_hash_mainnet() {
        let expected = felt!("0x70c7b342f93155315d1cb2da7a4e13a3c2430f51fb5696c1b224c3da5508dfb");
        let chain = MAINNET;

        let computed = compute_starknet_os_config_hash(chain, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }

    #[test]
    fn calculate_config_hash_testnet() {
        let expected = felt!("0x1b9900f77ff5923183a7795fcfbb54ed76917bc1ddd4160cc77fa96e36cf8c5");
        let chain = SEPOLIA;
        let computed = compute_starknet_os_config_hash(chain, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }
}
