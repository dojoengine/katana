use std::{borrow::Cow, time::Duration};

use crate::error::ProverError;
use reqwest::{
    multipart::{Form, Part},
    Client, ClientBuilder,
};
use serde::Deserialize;
use url::Url;

const ATLANTIC_API_BASE: &str = "https://atlantic.api.herodotus.cloud/";
const ATLANTIC_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticQueryResponse {
    pub atlantic_query: AtlanticQuery,
    pub metadata_urls: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticQuery {
    pub id: String,
    pub status: AtlanticQueryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticJobStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticJobSize {
    XS,
    S,
    M,
    L,
}

impl AtlanticJobSize {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::XS => "XS",
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AtlanticCairoVersion {
    Cairo0,
    Cairo1,
}

impl AtlanticCairoVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cairo0 => "cairo0",
            Self::Cairo1 => "cairo1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AtlanticCairoVmVersion {
    Rust,
    Python,
}

impl AtlanticCairoVmVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryResult {
    TraceGeneration,
    ProofGeneration,
    ProofVerificationOnL1,
    ProofVerificationOnL2,
}

impl AtlanticQueryResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TraceGeneration => "TRACE_GENERATION",
            Self::ProofGeneration => "PROOF_GENERATION",
            Self::ProofVerificationOnL1 => "PROOF_VERIFICATION_ON_L1",
            Self::ProofVerificationOnL2 => "PROOF_VERIFICATION_ON_L2",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[allow(non_camel_case_types)]
pub enum Layout {
    plain,
    small,
    dex,
    recursive,
    starknet,
    starknet_with_keccak,
    recursive_large_output,
    recursive_with_poseidon,
    all_solidity,
    all_cairo,
    dynamic,
}

impl Layout {
    pub fn to_str(self) -> &'static str {
        match self {
            Layout::plain => "plain",
            Layout::small => "small",
            Layout::dex => "dex",
            Layout::recursive => "recursive",
            Layout::starknet => "starknet",
            Layout::starknet_with_keccak => "starknet_with_keccak",
            Layout::recursive_large_output => "recursive_large_output",
            Layout::recursive_with_poseidon => "recursive_with_poseidon",
            Layout::all_solidity => "all_solidity",
            Layout::all_cairo => "all_cairo",
            Layout::dynamic => "dynamic",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlanticProofGenerationResponse {
    atlantic_query_id: String,
}

impl AtlanticQueryResponse {
    pub async fn get_proof(&self, atlantic_client: &AtlanticClient) -> Result<String, ProverError> {
        let url = if let Some(url) = self
            .metadata_urls
            .iter()
            .find(|url| url.contains("proof.json"))
        {
            url
        } else {
            return Err(ProverError::MetadataFetch(format!(
                "No proof found for query: {}",
                self.atlantic_query.id
            )));
        };

        let response = atlantic_client.http_client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(ProverError::Prover(format!(
                "unsuccessful status code: {}\n{}",
                response.status(),
                response.text().await?
            )));
        }
        Ok(response.text().await?)
    }

    pub async fn get_pie(&self, atlantic_client: &AtlanticClient) -> Result<Vec<u8>, ProverError> {
        let url = if let Some(url) = self
            .metadata_urls
            .iter()
            .find(|url| url.contains("pie.cairo0.zip"))
        {
            url
        } else {
            return Err(ProverError::MetadataFetch(format!(
                "No pie found for query: {}",
                self.atlantic_query.id
            )));
        };

        let response = atlantic_client.http_client.get(url).send().await?;
        Ok(response.bytes().await?.to_vec())
    }
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

    pub async fn submit_proof_generation<T>(
        &self,
        compressed_pie: T,
        layout: Layout,
        label: String,
        atlantic_job_size: AtlanticJobSize,
    ) -> Result<String, ProverError>
    where
        T: Into<Cow<'static, [u8]>>,
    {
        let mut url = self.api_base.clone();
        url.path_segments_mut().unwrap().push("atlantic-query");
        url.query_pairs_mut().append_pair("apiKey", &self.api_key);

        let form = Form::new()
            .part(
                "pieFile",
                Part::bytes(compressed_pie)
                    .file_name("pie.zip")
                    .mime_str("application/zip")
                    .unwrap(),
            )
            .text("layout", layout.to_str())
            .text("externalId", label)
            .text("declaredJobSize", atlantic_job_size.as_str())
            .text("result", AtlanticQueryResult::ProofGeneration.as_str());

        let response = self.http_client.post(url).multipart(form).send().await?;
        if !response.status().is_success() {
            return Err(ProverError::Prover(format!(
                "unsuccessful status code: {}\n{}",
                response.status(),
                response.text().await?
            )));
        }

        let response = response.json::<AtlanticProofGenerationResponse>().await?;
        Ok(response.atlantic_query_id)
    }

    pub async fn submit_trace_generation<P, I>(
        &self,
        label: &str,
        program: P,
        input: I,
    ) -> Result<String, ProverError>
    where
        P: Into<Cow<'static, [u8]>>,
        I: Into<Cow<'static, [u8]>>,
    {
        let mut url = self.api_base.clone();
        url.path_segments_mut().unwrap().push("atlantic-query");
        url.query_pairs_mut().append_pair("apiKey", &self.api_key);
        let form = Form::new()
            .text("cairoVersion", AtlanticCairoVersion::Cairo0.as_str())
            .text("result", AtlanticQueryResult::TraceGeneration.as_str())
            .text("declaredJobSize", AtlanticJobSize::XS.as_str())
            .text("cairoVm", AtlanticCairoVmVersion::Python.as_str())
            .text("externalId", label.to_string())
            .part(
                "programFile",
                Part::bytes(program.into())
                    .file_name("program.json")
                    .mime_str("application/json")
                    .unwrap(),
            )
            .part(
                "inputFile",
                Part::bytes(input.into())
                    .file_name("input.json")
                    .mime_str("application/json")
                    .unwrap(),
            );
        let response = self.http_client.post(url).multipart(form).send().await?;
        if !response.status().is_success() {
            return Err(ProverError::Prover(format!(
                "unsuccessful status code: {}\n{}",
                response.status(),
                response.text().await?
            )));
        }
        let response = response.json::<AtlanticProofGenerationResponse>().await?;
        Ok(response.atlantic_query_id)
    }

    pub async fn get_atlantic_query(self, id: &str) -> Result<AtlanticQueryResponse, ProverError> {
        let mut url = self.api_base.clone();
        url.path_segments_mut()
            .unwrap()
            .push("atlantic-query")
            .push(id);
        let response = self.http_client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(ProverError::Prover(format!(
                "unsuccessful status code: {}",
                response.status()
            )));
        }
        let response = response.json::<AtlanticQueryResponse>().await?;
        Ok(response)
    }
}
