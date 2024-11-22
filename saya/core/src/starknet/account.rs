use serde::{Deserialize, Serialize};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag};
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StarknetAccountData {
    pub starknet_url: Url,
    #[serde(deserialize_with = "felt_string_deserializer")]
    pub chain_id: Felt,
    pub signer_address: Felt,
    pub signer_key: Felt,
}
pub type SayaStarknetAccount = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;

impl StarknetAccountData {
    pub fn get_starknet_account(&self) -> SayaStarknetAccount {
        let provider = JsonRpcClient::new(HttpTransport::new(self.starknet_url.clone()));
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(self.signer_key));

        let mut account = SingleOwnerAccount::new(
            provider,
            signer,
            self.signer_address,
            self.chain_id,
            ExecutionEncoding::New,
        );

        account.set_block_id(BlockId::Tag(BlockTag::Pending));
        account
    }
}
pub fn felt_string_deserializer<'de, D>(deserializer: D) -> Result<Felt, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    cairo_short_string_to_felt(&s).map_err(serde::de::Error::custom)
}
