use std::{borrow::Cow, time::Duration};

use anyhow::Result;
use reqwest::{
    multipart::{Form, Part},
    Client, ClientBuilder,
};
use serde::Deserialize;
use url::Url;

const ATLANTIC_API_BASE: &str = "https://atlantic.api.herodotus.cloud/v1";
const ATLANTIC_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct AtlanticClient {
    http_client: Client,
    api_base: Url,
    api_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStatus {
    Received,
    Done,
    Failed,
    InProgress,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlanticProofGenerationResponse {
    atlantic_query_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlanticQueryResponse {
    atlantic_query: AtlanticQuery,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlanticQuery {
    status: AtlanticQueryStatus,
}

impl AtlanticClient {
    pub fn new(api_key: String) -> Self {
        Self {
            http_client: ClientBuilder::new()
                .timeout(ATLANTIC_HTTP_TIMEOUT)
                .build()
                .unwrap(),
            api_base: Url::parse(ATLANTIC_API_BASE).unwrap(),
            api_key,
        }
    }

    pub async fn submit_proof_generation<T>(&self, compressed_pie: T) -> Result<String>
    where
        T: Into<Cow<'static, [u8]>>,
    {
        let mut url = self.api_base.clone();
        url.path_segments_mut().unwrap().push("proof-generation");
        url.query_pairs_mut().append_pair("apiKey", &self.api_key);

        let form = Form::new()
            .part(
                "pieFile",
                Part::bytes(compressed_pie)
                    .file_name("pie.zip")
                    .mime_str("application/zip")
                    .unwrap(),
            )
            .text("layout", "dynamic")
            .text("prover", "starkware_sharp");

        let response = self.http_client.post(url).multipart(form).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("unsuccessful status code: {}", response.status());
        }

        let response = response.json::<AtlanticProofGenerationResponse>().await?;
        Ok(response.atlantic_query_id)
    }

    pub async fn get_query_status(&self, id: &str) -> Result<AtlanticQueryStatus> {
        let mut url = self.api_base.clone();
        url.path_segments_mut()
            .unwrap()
            .push("atlantic-query")
            .push(id);

        let response = self.http_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("unsuccessful status code: {}", response.status());
        }

        let response = response.json::<AtlanticQueryResponse>().await?;
        Ok(response.atlantic_query.status)
    }

    pub async fn get_proof(&self, id: &str) -> Result<String> {
        let url = format!(
            "https://atlantic-queries.s3.nl-ams.scw.cloud/sharp_queries/query_{}/proof.json",
            id
        );

        let response = self.http_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("unsuccessful status code: {}", response.status());
        }

        Ok(response.text().await?)
    }
}
