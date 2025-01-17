use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
pub struct Sharding {}

impl Sharding {
    pub async fn run(self) -> Result<()> {
        Err(anyhow::anyhow!(
            "sharding execution mode not yet implemented"
        ))
    }
}
