use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;
use headless_chrome::browser::default_executable;
use headless_chrome::{Browser, LaunchOptionsBuilder};
use katana_utils::node::test_config;
use katana_utils::TestNode;

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
    let _node = start_katana().await;
    let rp_handle = start_reverse_proxy().await;
    let browser = browser();

    // Test both direct and proxied endpoints
    let url = format!("http://localhost:{PORT}/explorer");
    test_all_pages(&browser, &url).await;

    let rp_url = format!("https://localhost:{RP_PORT}/x/foo/katana/explorer");
    test_all_pages(&browser, &rp_url).await;

    rp_handle.join().unwrap().kill().expect("failed to clean up reverse proxy")
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

async fn start_katana() -> TestNode {
    let mut config = test_config();
    config.rpc.explorer = true;
    // must match the port in the reverse proxy config (ie fixtures/Caddyfile)
    config.rpc.port = 6060;

    let node = TestNode::new_with_config(config).await;
    println!("Katana started");
    node
}

async fn start_reverse_proxy() -> JoinHandle<Child> {
    let caddy_check = Command::new("caddy").arg("--version").output();
    if caddy_check.is_err() || !caddy_check.unwrap().status.success() {
        panic!("Caddy is not installed.");
    }

    let has_started = Arc::new((Mutex::new(false), Condvar::new()));
    let has_started2 = Arc::clone(&has_started);

    let handle = thread::spawn(move || {
        let config_path = get_caddy_config_file_path();

        let handle = if Command::new("sudo").arg("--version").output().is_ok() {
            Command::new("sudo")
                .args(&["caddy", "run", "--config", config_path.to_str().unwrap()])
                .spawn()
                .expect("failed to start reverse proxy")
        } else {
            Command::new("caddy")
                .args(&["run", "--config", config_path.to_str().unwrap()])
                .spawn()
                .expect("failed to start reverse proxy")
        };

        let client = reqwest::blocking::Client::new();
        let health_check_url = format!("http://localhost:{RP_PORT}/health-check");

        for _ in 0..30 {
            if client.get(&health_check_url).send().is_ok() {
                println!("Reverse proxy server started");

                let (lock, cvar) = &*has_started2;
                let mut started = lock.lock().unwrap();
                *started = true;
                cvar.notify_one();

                return handle;
            }
            thread::sleep(Duration::from_secs(1));
        }

        panic!("timeout waiting on caddy to start: {health_check_url}");
    });

    // Wait for the thread to start up.
    let (lock, cvar) = &*has_started;
    let mut started = lock.lock().unwrap();
    while !*started {
        started = cvar.wait(started).unwrap();
    }

    handle
}

fn get_caddy_config_file_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/Caddyfile").canonicalize().unwrap()
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
