use std::time::Duration;

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use log::info;

use crate::prover::atlantic::{AtlanticClient, AtlanticJobStatus};

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const TRACE_GENERATION_JOB_NAME: &str = "TRACE_GENERATION";

#[derive(Debug)]
pub struct AtlanticTraceGenerator {
    pub atlantic_client: AtlanticClient,
}
impl AtlanticTraceGenerator {
    pub fn new(atlantic_client: AtlanticClient) -> Self {
        Self { atlantic_client }
    }
}

impl AtlanticTraceGenerator {
    pub async fn generate_trace(&self, program: Vec<u8>, input: Vec<u8>) -> Result<CairoPie> {
        let atlantic_query_id = self
            .atlantic_client
            .submit_trace_generation(program, input)
            .await?;
        info!(
            "Atlantic trace generation response: {:?}",
            atlantic_query_id
        );
        loop {
            tokio::time::sleep(PROOF_STATUS_POLL_INTERVAL).await;

            // TODO: error handling
            if let Ok(jobs) = self
                .atlantic_client
                .get_query_jobs(&atlantic_query_id)
                .await
            {
                if let Some(proof_generation_job) = jobs
                    .iter()
                    .find(|job| job.job_name == TRACE_GENERATION_JOB_NAME)
                {
                    match proof_generation_job.status {
                        AtlanticJobStatus::Completed => break,
                        AtlanticJobStatus::Failed => {
                            // TODO: error handling
                            panic!("Atlantic proof generation {} failed", atlantic_query_id);
                        }
                        AtlanticJobStatus::InProgress => {}
                    }
                }
            }
        }
        let pie_bytes = self.atlantic_client.get_trace(&atlantic_query_id).await?;
        let pie = CairoPie::from_bytes(&pie_bytes)?;
        info!("Trace generated for query: {}", atlantic_query_id);
        Ok(pie)
    }
}
