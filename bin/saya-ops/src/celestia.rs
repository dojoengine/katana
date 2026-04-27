use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use celestia_rpc::{BlobClient, Client};
use celestia_types::{blob::Commitment, nmt::Namespace};
use clap::{Args, Parser, Subcommand};
use tracing::info;

#[derive(Debug, Parser)]
pub struct Celestia {
    #[clap(subcommand)]
    cmd: CelestiaCmd,
}

#[derive(Debug, Subcommand)]
pub enum CelestiaCmd {
    /// Get a blob from Celestia
    BlobGet(BlobGetArgs),
    /// Convert a namespace string to hex, base64, and namespace ID
    Namespace(NamespaceArgs),
}

#[derive(Debug, Args)]
pub struct BlobGetArgs {
    /// Celestia RPC URL
    #[clap(long, env = "CELESTIA_RPC_URL")]
    rpc_url: String,
    /// Celestia auth token (optional)
    #[clap(long, env = "CELESTIA_AUTH_TOKEN")]
    auth_token: Option<String>,
    /// Block height
    #[clap(long)]
    height: u64,
    /// Namespace in hex format (e.g., "0x0102030405060708090a0b0c0d0e0f1011121314")
    #[clap(long, conflicts_with = "namespace_base64")]
    namespace_hex: Option<String>,
    /// Namespace in base64 format
    #[clap(long, conflicts_with = "namespace_hex")]
    namespace_base64: Option<String>,
    /// Commitment in hex format
    #[clap(long)]
    commitment: String,
}

#[derive(Debug, Args)]
pub struct NamespaceArgs {
    /// Namespace string (will be converted to v0 namespace)
    namespace: String,
}

impl Celestia {
    pub async fn run(self) -> Result<()> {
        match self.cmd {
            CelestiaCmd::BlobGet(args) => {
                blob_get(args).await?;
            }
            CelestiaCmd::Namespace(args) => {
                namespace_info(args)?;
            }
        }
        Ok(())
    }
}

async fn blob_get(args: BlobGetArgs) -> Result<()> {
    let namespace = if let Some(hex) = args.namespace_hex {
        let hex_str = hex.strip_prefix("0x").unwrap_or(&hex);
        let bytes = hex::decode(hex_str)?;
        Namespace::from_raw(&bytes)?
    } else if let Some(base64) = args.namespace_base64 {
        let bytes = STANDARD.decode(base64)?;
        Namespace::from_raw(&bytes)?
    } else {
        anyhow::bail!("Either --namespace-hex or --namespace-base64 must be provided");
    };

    let commitment_hex = args
        .commitment
        .strip_prefix("0x")
        .unwrap_or(&args.commitment);
    let commitment_bytes: [u8; 32] = hex::decode(commitment_hex)?
        .try_into()
        .expect("Invalid commitment");
    let commitment = Commitment::new(commitment_bytes);

    info!(
        "Fetching blob from Celestia:\n  Height: {}\n  Namespace: {}\n  Commitment: {}",
        args.height,
        STANDARD.encode(namespace.as_bytes()),
        hex::encode(commitment.hash())
    );

    let client = Client::new(&args.rpc_url, args.auth_token.as_deref(), None, None).await?;

    let blob = client.blob_get(args.height, namespace, commitment).await?;

    info!("Blob retrieved:");
    info!("  Data size: {} bytes", blob.data.len());
    info!("  Commitment: {}", hex::encode(blob.commitment.hash()));
    info!("  Share version: {:?}", blob.share_version);
    info!("  Namespace version: {:?}", blob.namespace.version());

    println!("\n=== Blob Data ===");
    println!("Hex: {}", hex::encode(&blob.data));
    println!("\nBase64: {}", STANDARD.encode(&blob.data));

    if let Ok(s) = String::from_utf8(blob.data.clone()) {
        println!("\nUTF-8: {}", s);
    }

    Ok(())
}

fn namespace_info(args: NamespaceArgs) -> Result<()> {
    let namespace = Namespace::new_v0(args.namespace.as_bytes())?;

    let raw_bytes = namespace.as_bytes();
    let hex_str = hex::encode(raw_bytes);
    let base64_str = STANDARD.encode(raw_bytes);
    let namespace_id = hex::encode(&raw_bytes[raw_bytes.len() - 10..]);

    info!("Namespace information for: '{}'", args.namespace);
    println!("Version: {:?}", namespace.version());
    println!("Hex: 0x{}", hex_str);
    println!("Base64: {}", base64_str);
    println!("Namespace ID (last 10 bytes): 0x{}", namespace_id);
    println!("Raw bytes length: {}", raw_bytes.len());

    Ok(())
}
