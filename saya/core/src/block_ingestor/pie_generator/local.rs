use anyhow::Result;
use cairo_vm::{types::layout_name::LayoutName, vm::runners::cairo_pie::CairoPie};

#[derive(Debug, Clone)]
pub struct LocalPieGenerator;

impl LocalPieGenerator {
    pub async fn prove_block(
        &self,
        snos: &[u8],
        block_number: u64,
        rpc_url: &str,
    ) -> Result<CairoPie> {
        prove_block::prove_block(snos, block_number, rpc_url, LayoutName::all_cairo, true)
            .await
            .map(|(pie, _)| pie)
            .map_err(|err| anyhow::anyhow!("{}", err))
    }
}
