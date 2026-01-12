//! AMD Key Distribution Service (KDS) Client
//!
//! Interfaces with AMD's public KDS API to retrieve certificates:
//! - Certificate chain (ARK + ASK)
//! - VCEK certificates

use crate::error::Error;
use crate::types::{
    AttestationCerts, CertChain, ChipId, ProcessorType, TcbVersion, VcekCert, VcekRequest,
};
use reqwest::Client;
use tracing::{debug, instrument};

/// AMD KDS base URL
pub const KDS_BASE_URL: &str = "https://kdsintf.amd.com";

/// AMD KDS client for fetching certificates
#[derive(Debug, Clone)]
pub struct KdsClient {
    client: Client,
    base_url: String,
}

impl Default for KdsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl KdsClient {
    /// Create a new KDS client with default settings
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: KDS_BASE_URL.to_string(),
        }
    }

    /// Create a new KDS client with a custom base URL (useful for testing)
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Create a new KDS client with a custom reqwest client
    pub fn with_client(client: Client) -> Self {
        Self {
            client,
            base_url: KDS_BASE_URL.to_string(),
        }
    }

    // ========================================================================
    // Certificate Chain (ARK + ASK)
    // ========================================================================

    /// Fetch the certificate chain (ARK + ASK) for a processor type
    ///
    /// # Arguments
    /// * `processor` - The processor type (Milan, Genoa, Bergamo, Siena)
    ///
    /// # Returns
    /// * `CertChain` containing ARK and ASK certificates in DER format
    ///
    /// # Example
    /// ```no_run
    /// use amd_tee_registry_client::{KdsClient, ProcessorType};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = KdsClient::new();
    /// let chain = client.fetch_cert_chain(ProcessorType::Milan).await?;
    /// println!("ARK size: {} bytes", chain.ark.len());
    /// println!("ASK size: {} bytes", chain.ask.len());
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self))]
    pub async fn fetch_cert_chain(&self, processor: ProcessorType) -> Result<CertChain, Error> {
        let url = format!(
            "{}/vcek/v1/{}/cert_chain",
            self.base_url,
            processor.as_kds_path()
        );

        debug!("Fetching certificate chain from: {}", url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(Error::KdsError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let pem_data = response.text().await?;
        self.parse_cert_chain_pem(&pem_data)
    }

    /// Parse PEM-encoded certificate chain into DER-encoded ARK and ASK
    fn parse_cert_chain_pem(&self, pem_data: &str) -> Result<CertChain, Error> {
        let pems: Vec<pem::Pem> = pem::parse_many(pem_data)?;

        if pems.len() < 2 {
            return Err(Error::CertificateParse(format!(
                "Expected at least 2 certificates in chain, got {}",
                pems.len()
            )));
        }

        // KDS returns certificates in this order:
        // 1. ASK (AMD SEV Key) - intermediate, signed by ARK
        // 2. ARK (AMD Root Key) - root, self-signed
        let ask = pems[0].contents().to_vec();
        let ark = pems[1].contents().to_vec();

        debug!(
            "Parsed ARK ({} bytes) and ASK ({} bytes)",
            ark.len(),
            ask.len()
        );

        Ok(CertChain { ark, ask })
    }

    // ========================================================================
    // VCEK Certificate
    // ========================================================================

    /// Fetch the VCEK certificate for a specific chip and TCB version
    ///
    /// # Arguments
    /// * `request` - VCEK request parameters (processor, chip_id, tcb)
    ///
    /// # Returns
    /// * `VcekCert` containing the VCEK certificate in DER format
    ///
    /// # Example
    /// ```no_run
    /// use amd_tee_registry_client::{KdsClient, ProcessorType, ChipId, TcbVersion, VcekRequest};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = KdsClient::new();
    ///
    /// let tcb = TcbVersion {
    ///     bootloader: 3,
    ///     tee: 0,
    ///     reserved: 0,
    ///     snp: 8,
    ///     microcode: 115,
    /// };
    ///
    /// let chip_id = ChipId::from_hex("0" .repeat(128).as_str())?;
    ///
    /// let request = VcekRequest::new(ProcessorType::Milan, chip_id, tcb);
    /// let vcek = client.fetch_vcek(&request).await?;
    /// println!("VCEK size: {} bytes", vcek.der.len());
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self))]
    pub async fn fetch_vcek(&self, request: &VcekRequest) -> Result<VcekCert, Error> {
        let url = format!(
            "{}/vcek/v1/{}/{}",
            self.base_url,
            request.processor.as_kds_path(),
            request.chip_id.to_hex()
        );

        debug!("Fetching VCEK from: {}", url);

        let response = self
            .client
            .get(&url)
            .query(&request.query_params())
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::KdsError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        // VCEK is returned in DER format
        let der = response.bytes().await?.to_vec();

        debug!("Fetched VCEK ({} bytes)", der.len());

        Ok(VcekCert {
            der,
            processor: request.processor,
            chip_id: request.chip_id.clone(),
            tcb: request.tcb,
        })
    }

    // ========================================================================
    // Convenience Methods
    // ========================================================================

    /// Fetch all certificates needed for attestation verification
    ///
    /// This fetches both the certificate chain (ARK + ASK) and the VCEK
    /// in parallel for efficiency.
    ///
    /// # Arguments
    /// * `request` - VCEK request parameters
    ///
    /// # Returns
    /// * `AttestationCerts` containing the complete certificate bundle
    #[instrument(skip(self))]
    pub async fn fetch_attestation_certs(
        &self,
        request: &VcekRequest,
    ) -> Result<AttestationCerts, Error> {
        // Fetch cert chain and VCEK in parallel
        let (chain, vcek) = tokio::try_join!(
            self.fetch_cert_chain(request.processor),
            self.fetch_vcek(request)
        )?;

        Ok(AttestationCerts { chain, vcek })
    }

    /// Fetch VCEK using raw parameters
    ///
    /// Convenience method that creates a VcekRequest internally.
    pub async fn fetch_vcek_raw(
        &self,
        processor: ProcessorType,
        chip_id: ChipId,
        bootloader: u8,
        tee: u8,
        snp: u8,
        microcode: u8,
    ) -> Result<VcekCert, Error> {
        let tcb = TcbVersion {
            bootloader,
            tee,
            reserved: 0,
            snp,
            microcode,
        };
        let request = VcekRequest::new(processor, chip_id, tcb);
        self.fetch_vcek(&request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vcek_request_query_params() {
        let tcb = TcbVersion {
            bootloader: 3,
            tee: 0,
            reserved: 0,
            snp: 8,
            microcode: 115,
        };
        let chip_id = ChipId::new([0u8; 64]);
        let request = VcekRequest::new(ProcessorType::Milan, chip_id, tcb);

        let params = request.query_params();
        assert_eq!(params.len(), 4);
        assert_eq!(params[0], ("blSPL", "3".to_string()));
        assert_eq!(params[1], ("teeSPL", "0".to_string()));
        assert_eq!(params[2], ("snpSPL", "8".to_string()));
        assert_eq!(params[3], ("ucodeSPL", "115".to_string()));
    }

    #[test]
    fn test_processor_type_kds_path() {
        assert_eq!(ProcessorType::Milan.as_kds_path(), "Milan");
        assert_eq!(ProcessorType::Genoa.as_kds_path(), "Genoa");
        assert_eq!(ProcessorType::Bergamo.as_kds_path(), "Bergamo");
        assert_eq!(ProcessorType::Siena.as_kds_path(), "Siena");
    }

    #[test]
    fn test_chip_id_hex() {
        let mut bytes = [0u8; 64];
        bytes[0] = 0xAB;
        bytes[63] = 0xCD;
        let chip_id = ChipId::new(bytes);

        let hex = chip_id.to_hex();
        assert_eq!(hex.len(), 128);
        assert!(hex.starts_with("ab"));
        assert!(hex.ends_with("cd"));

        let parsed = ChipId::from_hex(&hex).unwrap();
        assert_eq!(*parsed.as_bytes(), bytes);
    }

    #[test]
    fn test_tcb_version_bytes() {
        let tcb = TcbVersion {
            bootloader: 3,
            tee: 0,
            reserved: 0,
            snp: 8,
            microcode: 115,
        };

        let bytes = tcb.to_bytes();
        let parsed = TcbVersion::from_bytes(&bytes);

        assert_eq!(parsed.bootloader, 3);
        assert_eq!(parsed.tee, 0);
        assert_eq!(parsed.reserved, 0);
        assert_eq!(parsed.snp, 8);
        assert_eq!(parsed.microcode, 115);
    }

    // ========================================================================
    // Integration tests - require network access
    // Run with: cargo test -- --ignored
    // ========================================================================

    /// Helper to verify a certificate chain has valid structure
    fn verify_cert_chain(chain: &super::CertChain, processor: ProcessorType) {
        // ARK (AMD Root Key) should be a valid DER-encoded certificate
        assert!(
            !chain.ark.is_empty(),
            "{}: ARK certificate is empty",
            processor
        );
        assert!(
            chain.ark.len() > 100,
            "{}: ARK certificate too small ({} bytes)",
            processor,
            chain.ark.len()
        );

        // ASK (AMD SEV Key) should be a valid DER-encoded certificate
        assert!(
            !chain.ask.is_empty(),
            "{}: ASK certificate is empty",
            processor
        );
        assert!(
            chain.ask.len() > 100,
            "{}: ASK certificate too small ({} bytes)",
            processor,
            chain.ask.len()
        );

        // DER certificates typically start with 0x30 (SEQUENCE tag)
        assert_eq!(
            chain.ark[0], 0x30,
            "{}: ARK doesn't start with SEQUENCE tag",
            processor
        );
        assert_eq!(
            chain.ask[0], 0x30,
            "{}: ASK doesn't start with SEQUENCE tag",
            processor
        );

        println!(
            "{}: ARK={} bytes, ASK={} bytes",
            processor,
            chain.ark.len(),
            chain.ask.len()
        );
    }

    #[tokio::test]
    async fn test_fetch_cert_chain_milan() {
        let client = KdsClient::new();
        let chain = client
            .fetch_cert_chain(ProcessorType::Milan)
            .await
            .expect("Failed to fetch Milan cert chain");
        verify_cert_chain(&chain, ProcessorType::Milan);
    }

    #[tokio::test]
    async fn test_fetch_cert_chain_genoa() {
        let client = KdsClient::new();
        let chain = client
            .fetch_cert_chain(ProcessorType::Genoa)
            .await
            .expect("Failed to fetch Genoa cert chain");
        verify_cert_chain(&chain, ProcessorType::Genoa);
    }

    // NOTE: Bergamo and Siena are not yet available on AMD KDS (returns 404)
    // These tests verify the expected 404 behavior
    #[tokio::test]
    async fn test_fetch_cert_chain_bergamo_not_available() {
        let client = KdsClient::new();
        let result = client.fetch_cert_chain(ProcessorType::Bergamo).await;

        // Bergamo is not yet available on AMD KDS
        match result {
            Err(Error::KdsError { status: 404, .. }) => {
                println!("Bergamo: Not available on KDS (404) - expected");
            }
            Ok(chain) => {
                // If AMD adds support, this test will catch it
                verify_cert_chain(&chain, ProcessorType::Bergamo);
                println!("Bergamo: Now available on KDS!");
            }
            Err(e) => panic!("Unexpected error for Bergamo: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_fetch_cert_chain_siena_not_available() {
        let client = KdsClient::new();
        let result = client.fetch_cert_chain(ProcessorType::Siena).await;

        // Siena is not yet available on AMD KDS
        match result {
            Err(Error::KdsError { status: 404, .. }) => {
                println!("Siena: Not available on KDS (404) - expected");
            }
            Ok(chain) => {
                // If AMD adds support, this test will catch it
                verify_cert_chain(&chain, ProcessorType::Siena);
                println!("Siena: Now available on KDS!");
            }
            Err(e) => panic!("Unexpected error for Siena: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_fetch_all_available_cert_chains() {
        let client = KdsClient::new();

        // Currently only Milan and Genoa are available on AMD KDS
        let available_processors = [ProcessorType::Milan, ProcessorType::Genoa];

        for processor in available_processors {
            let chain = client
                .fetch_cert_chain(processor)
                .await
                .expect(&format!("Failed to fetch {} cert chain", processor));
            verify_cert_chain(&chain, processor);
        }
    }
}
