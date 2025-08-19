//! A test to ensure the Explorer web app works correctly even when accessed through the reverse
//! proxy.
//!
//! This test assumes that Katana and the reverse proxy are already running on port 6060 and 9090
//! respectively.

use anyhow::Result;
use headless_chrome::browser::default_executable;
use headless_chrome::{Browser, LaunchOptionsBuilder};
use reqwest::Client;

// must match the values in fixtures/Caddyfile
const PORT: u16 = 6060;
const RP_PORT: u16 = 9090;

// (name, route, value, id selector)
const ROUTES: [(&str, &str, Option<&str>, &str); 5] = [
    ("Home", "/", None, "home-search-bar"),
    ("Block Details", "/block", Some("0"), "block-details"),
    (
        "Class Hash Details",
        "/class",
        Some("0x07dc7899aa655b0aae51eadff6d801a58e97dd99cf4666ee59e704249e51adf2"),
        "class-details",
    ),
    (
        "Contract details",
        "/contract",
        Some("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
        "contract-details",
    ),
    ("JSON Playground", "/jrpc", None, "json-playground"),
    // ("Transaction Details", "tx", Some("0x0"), "tx-details"),
];

#[tokio::main]
async fn main() {
    // Check if the direct URL is healthy before running tests
    if !check_url_health(&format!("http://localhost:{PORT}/"), false).await {
        panic!(
            "❌ Failed to connect to Katana at {PORT}. Please make sure Katana is already running."
        );
    }

    // Check if the reverse proxy URL is healthy before running tests
    if !check_url_health(&format!("https://localhost:{RP_PORT}/health-check"), true).await {
        panic!(
            "❌ Failed to connect to the reverse proxy at port {RP_PORT}. Please make sure the \
             reverse proxy is already running."
        );
    }

    let url = format!("http://localhost:{PORT}/explorer");
    let rp_url = format!("https://localhost:{RP_PORT}/x/foo/katana/explorer");

    let browser = browser();

    // Test both direct and proxied endpoints
    test_all_pages(&browser, &url).await;
    test_all_pages(&browser, &rp_url).await;
}

async fn check_url_health(url: &str, use_https: bool) -> bool {
    let client = if use_https {
        Client::builder().danger_accept_invalid_certs(true).build().unwrap()
    } else {
        Client::new()
    };

    match client.get(url).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

async fn test_all_pages(browser: &Browser, base_url: &str) {
    println!("Testing pages with base URL: {base_url}");
    for route in ROUTES {
        let (name, route, value, selector) = route;

        let url = if let Some(val) = value {
            format!("{base_url}{route}/{val}")
        } else {
            format!("{base_url}{route}")
        };

        test_page(browser, name, &url, selector).unwrap();
    }
}

fn test_page(browser: &Browser, page_name: &str, url: &str, selector: &str) -> Result<()> {
    println!("Testing {} page at {}", page_name, url);

    let tab = browser.new_tab()?;
    let tab = tab.navigate_to(url)?;

    // Wait for the page-specific element to appear
    let element_id = format!("#{selector}");
    match tab.wait_for_element(&element_id) {
        Ok(_) => {
            println!("✅ Successfully loaded {page_name} page");
            Ok(())
        }
        Err(e) => {
            println!("❌ Failed to load {page_name} page: {e}");
            Err(e)
        }
    }
}

fn browser() -> Browser {
    let mut builder = LaunchOptionsBuilder::default();
    builder.path(Some(default_executable().expect("no chrome executable found")));

    // Chrome disallows running in no-sandbox (the default) mode as root (when run in ci)
    if nix::unistd::geteuid().is_root() {
        builder.sandbox(false);
    }

    let opts = builder.build().unwrap();
    Browser::new(opts).expect("failed to create browser")
}
