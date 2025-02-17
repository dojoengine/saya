use super::BlockPieGenerator;
use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use local::LocalPieGenerator;
use remote::RemotePieGenerator;

pub mod local;
pub mod remote;

#[derive(Debug)]
pub enum SnosPieGenerator {
    Local(LocalPieGenerator),
    Remote(Box<RemotePieGenerator>),
}

impl BlockPieGenerator for SnosPieGenerator {
    async fn prove_block(&self, snos: &[u8], block_number: u64, rpc_url: &str) -> Result<CairoPie> {
        match self {
            Self::Local(inner) => inner.prove_block(snos, block_number, rpc_url).await,
            Self::Remote(inner) => inner.prove_block(snos, block_number, rpc_url).await,
        }
    }
}
