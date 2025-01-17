use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
pub struct Persistent {}

impl Persistent {
    pub async fn run(self) -> Result<()> {
        Err(anyhow::anyhow!("persistent L3 mode not yet implemented"))
    }
}
