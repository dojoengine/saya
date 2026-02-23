use starknet::core::types::Felt;

/// RPC URL of the settlement (L2) chain.
/// Defaults to the port exposed by `katana_bootstrap` in `compose.yml`.
pub fn settlement_rpc_url() -> String {
    std::env::var("SETTLEMENT_RPC_URL").unwrap_or_else(|_| "http://localhost:5050".to_string())
}

/// Address of the deployed piltover core contract.
/// Defaults to the address used in `compose.yml`.
pub fn piltover_address() -> Felt {
    let addr = std::env::var("PILTOVER_ADDRESS").unwrap_or_else(|_| {
        "0x1c8a55203cd99a6bfaf7cd91ae2ad953eff67b584826edab1857ca2e3c5db5d".to_string()
    });
    Felt::from_hex(&addr).expect("invalid PILTOVER_ADDRESS")
}

/// Chain ID used when configuring the core contract program info.
pub fn chain_id() -> String {
    std::env::var("CHAIN_ID").unwrap_or_else(|_| "custom".to_string())
}

/// Fact registry address configured for the core contract.
pub fn fact_registry_address() -> Felt {
    let addr = std::env::var("FACT_REGISTRY_ADDRESS").unwrap_or_else(|_| {
        "0x3eb0d510d1238120bf7f9d176faafe0c7066797a86be985855952f87769d3bd".to_string()
    });
    Felt::from_hex(&addr).expect("invalid FACT_REGISTRY_ADDRESS")
}

/// Fee token address used to build the Starknet OS config hash.
pub fn fee_token_address() -> Felt {
    let addr = std::env::var("FEE_TOKEN_ADDRESS").unwrap_or_else(|_| {
        "0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea".to_string()
    });
    Felt::from_hex(&addr).expect("invalid FEE_TOKEN_ADDRESS")
}
