use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{anyhow, Result};
use bytes::Bytes;
use http::header::HeaderValue;
use http::{Request, Response, StatusCode};
use rust_embed::RustEmbed;
use tower::{Layer, Service};

// Define Body type based on what's available
#[cfg(feature = "jsonrpsee")]
use jsonrpsee::core::{http_helpers::Body, BoxError};

#[cfg(not(feature = "jsonrpsee"))]
type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[cfg(not(feature = "jsonrpsee"))]
#[derive(Debug)]
pub enum Body {
    Fixed(Bytes),
}

#[cfg(not(feature = "jsonrpsee"))]
impl Body {
    fn from<T: Into<Vec<u8>>>(data: T) -> Self {
        Self::Fixed(Bytes::from(data.into()))
    }
}

#[cfg(not(feature = "jsonrpsee"))]
impl http_body::Body for Body {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Body::Fixed(bytes) if !bytes.is_empty() => {
                let frame = http_body::Frame::data(std::mem::take(bytes));
                std::task::Poll::Ready(Some(Ok(frame)))
            }
            _ => std::task::Poll::Ready(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExplorerLayer {
    /// The chain ID of the network
    chain_id: String,
}

impl ExplorerLayer {
    pub fn new(chain_id: String) -> Result<Self> {
        // Validate that the embedded assets are available
        if ExplorerAssets::get("index.html").is_none() {
            return Err(anyhow!(
                "Explorer assets not found. Make sure the explorer UI is built in CI and the \
                 ui/dist directory is available."
            ));
        }

        Ok(Self { chain_id })
    }
}

impl<S> Layer<S> for ExplorerLayer {
    type Service = ExplorerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ExplorerService { inner, chain_id: self.chain_id.clone() }
    }
}

#[derive(Debug, Clone)]
pub struct ExplorerService<S> {
    inner: S,
    chain_id: String,
}

impl<S, B> Service<Request<B>> for ExplorerService<S>
where
    B::Data: Send,
    S::Response: 'static,
    B::Error: Into<BoxError>,
    S::Future: Send + 'static,
    S::Error: Into<BoxError> + 'static,
    S: Service<Request<B>, Response = Response<Body>>,
    B: http_body::Body<Data = Bytes> + Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // If the path does not start with the base path, pass the request to the inner service.
        let Some(rel_path) = req.uri().path().strip_prefix("/explorer") else {
            return Box::pin(self.inner.call(req));
        };

        // Check if the request is for a static asset that actually exists
        let file_path = rel_path.trim_start_matches('/');
        let is_static_asset = is_static_asset_path(file_path);

        // If it's a static asset, try to find the exact file.
        // Otherwise, serve `index.html` since it's a SPA route.
        let asset_path = if is_static_asset && ExplorerAssets::get(file_path).is_some() {
            file_path.to_string()
        } else {
            "index.html".to_string()
        };

        let response = if let Some(asset) = ExplorerAssets::get(&asset_path) {
            let content_type = get_content_type(&format!("/{asset_path}"));
            let content = asset.data;

            let body = if content_type == "text/html" {
                let html = String::from_utf8_lossy(&content).to_string();
                let html = setup_env(&html, &self.chain_id);
                Body::from(html)
            } else {
                Body::from(content.to_vec())
            };

            let mut response = Response::builder().body(body).unwrap();

            let mut headers = req.headers().clone();
            let content_type = HeaderValue::from_str(content_type).unwrap();
            headers.insert("Content-Type", content_type);
            response.headers_mut().extend(headers);

            response
        } else {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not found"))
                .expect("good data; qed")
        };

        Box::pin(async { Ok(response) })
    }
}

/// Embedded explorer UI files.
#[derive(RustEmbed)]
#[folder = "ui/dist"]
struct ExplorerAssets;

/// This function adds a script tag to the HTML that sets up environment variables
/// for the explorer to use.
fn setup_env(html: &str, chain_id: &str) -> String {
    let escaped_chain_id = chain_id.replace("\"", "\\\"").replace("<", "&lt;").replace(">", "&gt;");

    // We inject the chain ID into the HTML for the controller to use.
    // The chain id is a required param to initialize the controller <https://github.com/cartridge-gg/controller/blob/main/packages/controller/src/controller.ts#L32>.
    // The parameters are consumed by the explorer here <https://github.com/cartridge-gg/explorer/blob/68ac4ea9500a90abc0d7c558440a99587cb77585/src/constants/rpc.ts#L14-L15>.

    // NOTE: ENABLE_CONTROLLER feature flag is a temporary solution to handle the controller.
    // The controller expects to have a `defaultChainId` but we don't have a way
    // to set it in the explorer yet in development mode (locally running katana instance).
    // The temporary solution is to disable the controller by setting the ENABLE_CONTROLLER flag to
    // false for these explorers. Once we have an updated controller JS SDK which can handle the
    // chain ID of local katana instances then we can remove this flag value. (ref - https://github.com/cartridge-gg/controller/blob/main/packages/controller/src/controller.ts#L57)
    // TODO: remove the ENABLE_CONTROLLER flag once we have a proper way to handle the chain ID for
    // local katana instances.
    let script = format!(
        r#"<script>
                window.CHAIN_ID = "{}";
                window.ENABLE_CONTROLLER = false;
            </script>"#,
        escaped_chain_id,
    );

    if let Some(head_pos) = html.find("<head>") {
        let (start, end) = html.split_at(head_pos + 6);
        format!("{}{}{}", start, script, end)
    } else {
        format!("{}\n{}", script, html)
    }
}

/// Gets the content type for a file based on its extension.
fn get_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        _ => "application/octet-stream",
    }
}

/// Checks if the given path is a path to a static asset.
fn is_static_asset_path(path: &str) -> bool {
    !path.is_empty()
        && (path.ends_with(".js")
            || path.ends_with(".css")
            || path.ends_with(".png")
            || path.ends_with(".svg")
            || path.ends_with(".json")
            || path.ends_with(".ico")
            || path.ends_with(".woff")
            || path.ends_with(".woff2")
            || path.ends_with(".ttf")
            || path.ends_with(".eot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_content_type() {
        assert_eq!(get_content_type("index.html"), "text/html");
        assert_eq!(get_content_type("app.js"), "application/javascript");
        assert_eq!(get_content_type("styles.css"), "text/css");
        assert_eq!(get_content_type("logo.png"), "image/png");
        assert_eq!(get_content_type("icon.svg"), "image/svg+xml");
        assert_eq!(get_content_type("data.json"), "application/json");
        assert_eq!(get_content_type("favicon.ico"), "image/x-icon");
        assert_eq!(get_content_type("font.woff"), "font/woff");
        assert_eq!(get_content_type("font.woff2"), "font/woff2");
        assert_eq!(get_content_type("font.ttf"), "font/ttf");
        assert_eq!(get_content_type("font.eot"), "application/vnd.ms-fontobject");
        assert_eq!(get_content_type("unknown.xyz"), "application/octet-stream");
        assert_eq!(get_content_type("no_extension"), "application/octet-stream");
    }

    #[test]
    fn test_is_static_asset_path() {
        // Valid static assets
        assert!(is_static_asset_path("app.js"));
        assert!(is_static_asset_path("styles.css"));
        assert!(is_static_asset_path("logo.png"));
        assert!(is_static_asset_path("icon.svg"));
        assert!(is_static_asset_path("data.json"));
        assert!(is_static_asset_path("favicon.ico"));
        assert!(is_static_asset_path("font.woff"));
        assert!(is_static_asset_path("font.woff2"));
        assert!(is_static_asset_path("font.ttf"));
        assert!(is_static_asset_path("font.eot"));

        // Nested paths
        assert!(is_static_asset_path("assets/js/app.js"));
        assert!(is_static_asset_path("css/main.css"));

        // Invalid static assets
        assert!(!is_static_asset_path(""));
        assert!(!is_static_asset_path("index.html"));
        assert!(!is_static_asset_path("page"));
        assert!(!is_static_asset_path("unknown.txt"));
        assert!(!is_static_asset_path("file.xml"));
    }

    #[test]
    fn test_setup_env_basic_injection() {
        let html = "<html><head></head><body></body></html>";
        let chain_id = "KATANA";
        let result = setup_env(html, chain_id);

        assert!(result.contains("window.CHAIN_ID = \"KATANA\";"));
        assert!(result.contains("window.ENABLE_CONTROLLER = false;"));
        assert!(result.contains("<head><script>"));
    }

    #[test]
    fn test_setup_env_escapes_chain_id() {
        let html = "<html><head></head><body></body></html>";

        // Test escaping quotes
        let chain_id = "CHAIN\"WITH\"QUOTES";
        let result = setup_env(html, chain_id);
        assert!(result.contains("window.CHAIN_ID = \"CHAIN\\\"WITH\\\"QUOTES\";"));

        // Test escaping HTML characters
        let chain_id = "CHAIN<WITH>TAGS";
        let result = setup_env(html, chain_id);
        assert!(result.contains("window.CHAIN_ID = \"CHAIN&lt;WITH&gt;TAGS\";"));
    }

    #[test]
    fn test_setup_env_no_head_tag() {
        let html = "<html><body></body></html>";
        let chain_id = "KATANA";
        let result = setup_env(html, chain_id);

        assert!(result.starts_with("<script>"));
        assert!(result.contains("window.CHAIN_ID = \"KATANA\";"));
    }

    #[test]
    fn test_setup_env_preserves_html_structure() {
        let html = "<html><head><title>Test</title></head><body><h1>Hello</h1></body></html>";
        let chain_id = "KATANA";
        let result = setup_env(html, chain_id);

        assert!(result.contains("<title>Test</title>"));
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("window.CHAIN_ID = \"KATANA\";"));
    }

    #[test]
    fn test_explorer_layer_new_with_assets() {
        // This test verifies that ExplorerLayer::new works when assets are available
        // The build.rs script may have built assets, so this might succeed
        let result = ExplorerLayer::new("test-chain".to_string());

        // The test can succeed or fail depending on whether assets were built
        // What matters is that we get the expected behavior in each case
        match result {
            Ok(layer) => {
                // If assets are available, we should get a layer with the correct chain_id
                assert_eq!(layer.chain_id, "test-chain");
            }
            Err(err) => {
                // If assets are not available, we should get the expected error
                assert!(err.to_string().contains("Explorer assets not found"));
            }
        }
    }
}
