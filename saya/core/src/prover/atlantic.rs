use atlantic_client::models::{AtlanticSdk, Layout, ProverVersion};
use swiftness_proof_parser::{parse, StarkProof};
use tracing::trace;
use url::Url;

use crate::db::sql_lite::SqliteDb;
use crate::db::ProverStatus;
use crate::errors::Error;

const LAYOUT_BRIDGE: &[u8; 31478586] =
    include_bytes!("../../../../bin/saya/programs/layout_bridge.json");
pub struct AtlanticProver {
    pub api_key: String,
    pub url: Url,
    pub db: SqliteDb,
}
pub type QueryId = String;

impl AtlanticProver {
    pub fn new(api_key: String, url: Url, db: SqliteDb) -> Self {
        AtlanticProver { api_key, url, db }
    }
    pub async fn submit_proof_generation(&self, pie: Vec<u8>) -> Result<QueryId, Error> {
        let sdk = AtlanticSdk::new(self.api_key.clone(), self.url.clone())?;
        let is_alive = sdk.get_is_alive().await?;
        if !is_alive {
            return Err(Error::ServerNotAliveError);
        }

        let id = sdk
            .proof_generation(pie, Layout::Dynamic, ProverVersion::Starkware)
            .await?
            .sharp_query_id;
        Ok(id)
    }
    pub async fn submit_atlantic_query(&self, proof: String) -> Result<QueryId, Error> {
        let sdk = AtlanticSdk::new(self.api_key.clone(), self.url.clone())?;
        let is_alive = sdk.get_is_alive().await?;
        if !is_alive {
            return Err(Error::ServerNotAliveError);
        }

        // We need to format the input as a json object because layout bridge expects a json object
        // with {"proof": proof}
        let input = format!("{{\n\t\"proof\": {}\n}}", proof);

        let id = sdk
            .l2_atlantic_query(
                LAYOUT_BRIDGE.to_vec(),
                input.as_bytes().to_vec(),
                ProverVersion::Starkware,
                false,
            )
            .await?
            .sharp_query_id;
        Ok(id)
    }

    pub async fn fetch_proof(&self, query_id: &str) -> Result<String, Error> {
        let proof_path = format!(
            "https://atlantic-queries.s3.nl-ams.scw.cloud/sharp_queries/query_{}/proof.json",
            query_id
        );
        let client = reqwest::Client::new();
        let response = client.get(&proof_path).send().await?;
        let response_text = response.text().await?;
        let _: StarkProof = parse(response_text.clone())?; //We just verify to see if its valid proof format
        Ok(response_text)
    }

    pub async fn check_query_status(&self, id: u32, query_id: &str) -> Result<ProverStatus, Error> {
        trace!("Checking status for block {}, query_id {}", id, query_id);

        let sdk = AtlanticSdk::new(self.api_key.clone(), self.url.clone())?;

        let is_alive = sdk.get_is_alive().await?;
        if !is_alive {
            return Err(Error::ServerNotAliveError);
        }

        trace!("Checking status for query_id {}", query_id);

        let job_response = sdk.get_sharp_query_jobs(query_id).await?;
        // Determine the ProverStatus
        if job_response.jobs.is_empty() {
            return Ok(ProverStatus::Proving); // No jobs found yet
        }
        if job_response.jobs.iter().any(|job| job.status == "FAILED") {
            return Ok(ProverStatus::Failed); // If any job failed
        }
        if job_response
            .jobs
            .iter()
            .all(|job| job.status == "COMPLETED")
        {
            return Ok(ProverStatus::Proved); // All jobs completed
        }
        Ok(ProverStatus::Proving)
    }
}
