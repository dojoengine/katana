use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{anyhow, Result};
use axum::body::Body;
use axum::http::{HeaderValue, Response};
use http::{Request, StatusCode};
use rust_embed::RustEmbed;
use tower::{Layer, Service};

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

#[derive(Debug)]
pub struct ExplorerService<S> {
    inner: S,
    chain_id: String,
}

impl<S> Service<Request<Body>> for ExplorerService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
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
