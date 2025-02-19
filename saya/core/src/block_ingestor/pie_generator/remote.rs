use anyhow::Result;
use cairo_vm::{types::layout_name::LayoutName, vm::runners::cairo_pie::CairoPie};
use prover_sdk::{
    access_key::ProverAccessKey, models::SnosPieOutput, sdk::ProverSDK, snos_input::SnosPieInput,
    JobResponse, JobResult,
};
#[derive(Debug, Clone)]
pub struct RemotePieGenerator {
    pub url: String,
    pub access_key: ProverAccessKey,
}

impl RemotePieGenerator {
    pub async fn prove_block(
        &self,
        snos: &[u8],
        block_number: u64,
        rpc_url: &str,
    ) -> Result<CairoPie> {
        // Parse URL and handle error
        let prover_url = self
            .url
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

        // Initialize Prover SDK
        let prover_sdk = prover_sdk::sdk::ProverSDK::new(prover_url, self.access_key.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize Prover SDK: {}", e))?;

        let snos_pie_input = SnosPieInput {
            compiled_os: snos.to_vec(),
            block_number,
            rpc_provider: rpc_url.to_string(),
            layout: LayoutName::all_cairo,
            full_output: true,
        };

        // Submit job
        let job = prover_sdk
            .snos_pie_gen(snos_pie_input)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit job: {}", e))?;

        // Wait for completion
        prover_sdk
            .sse(job)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to subscribe to job events: {}", e))?;

        let pie = fetch_and_process_job_result(&prover_sdk, job).await?;

        // Convert to CairoPie
        let cairo_pie = CairoPie::from_bytes(&pie.pie)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize CairoPie: {}", e))?;

        Ok(cairo_pie)
    }
}

async fn fetch_and_process_job_result(prover_sdk: &ProverSDK, job: u64) -> Result<SnosPieOutput> {
    let response = prover_sdk
        .get_job(job)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to retrieve job status: {}", e))?;

    let response_text = response
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to extract text from response: {}", e))?;

    let job_response: JobResponse = serde_json::from_str(&response_text)
        .map_err(|e| anyhow::anyhow!("Failed to parse job response JSON: {}", e))?;

    let result = match job_response {
        JobResponse::Completed { result, .. } => result,
        _ => return Err(anyhow::anyhow!("Job not completed successfully")),
    };

    handle_completed_job_response(result)
}

pub fn handle_completed_job_response(result: JobResult) -> Result<SnosPieOutput> {
    match result {
        JobResult::Prove(_) | JobResult::Run(_) => Err(anyhow::anyhow!(
            "Expected a prove result, but got a different result type"
        )),
        JobResult::Snos(run_result) => Ok(run_result),
    }
}
