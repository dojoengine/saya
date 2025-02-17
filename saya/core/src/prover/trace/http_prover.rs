use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use prover_sdk::sdk::ProverSDK;
use prover_sdk::{access_key::ProverAccessKey, Cairo0ProverInput};
use prover_sdk::{JobResponse, JobResult, Layout, RunResult};
#[derive(Debug)]
pub struct HttpProverTraceGen {
    pub url: String,
    pub access_key: ProverAccessKey,
}

impl HttpProverTraceGen {
    pub async fn generate_trace(&self, program: Vec<u8>, input: Vec<u8>) -> Result<CairoPie> {
        let prover_url = self
            .url
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
        let prover_sdk = ProverSDK::new(prover_url, self.access_key.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize Prover SDK: {}", e))?;
        let input = Cairo0ProverInput {
            program,
            program_input: input,
            layout: Layout::RecursiveWithPoseidon,
            n_queries: None,
            pow_bits: None,
            run_mode: prover_sdk::RunMode::Pie,
        };

        let job = prover_sdk.run_cairo0(input).await?;
        prover_sdk.sse(job).await?;
        let pie = fetch_and_process_job_result(&prover_sdk, job).await?;
        let pie = match pie {
            RunResult::Pie(pie) => pie,
            _ => panic!("Expected a pie result, but got a different result type"),
        };
        let cairo_pie = CairoPie::from_bytes(&pie)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize CairoPie: {}", e))?;
        Ok(cairo_pie)
    }
}

async fn fetch_and_process_job_result(prover_sdk: &ProverSDK, job: u64) -> Result<RunResult> {
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

pub fn handle_completed_job_response(result: JobResult) -> Result<RunResult> {
    match result {
        JobResult::Prove(_) | JobResult::Snos(_) => Err(anyhow::anyhow!(
            "Expected a run result, but got a different result type"
        )),
        JobResult::Run(run_result) => Ok(run_result),
    }
}
