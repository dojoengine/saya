use clap::Args;
use url::Url;

#[derive(Debug, Args, Clone)]
pub struct ProofOptions {
    #[arg(long)]
    #[arg(value_name = "PROVER URL")]
    #[arg(help = "The Prover URL for remote proving.")]
    pub prover_url: Url,

    #[arg(long)]
    #[arg(value_name = "PROVER KEY")]
    #[arg(help = "An authorized prover key for remote proving.")]
    pub private_key: String,
}
