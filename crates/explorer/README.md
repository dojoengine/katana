# Katana Explorer

The Katana Explorer provides a web interface for interacting with Katana blockchain instances. This crate implements a flexible Tower middleware for serving the Explorer UI with multiple deployment modes.

## Features

- 🔥 **Hot Reload**: Real-time UI updates during development
- 📦 **Embedded Assets**: Production-ready embedded UI 
- 🗂️ **Filesystem Serving**: Serve UI from any directory
- 🔒 **Security Headers**: Configurable security headers for production
- 🌐 **CORS Support**: Enable cross-origin requests for development
- ⚡ **Caching**: Intelligent file caching with invalidation
- 🎯 **SPA Support**: Single Page Application routing support

## Deployment Modes

### 1. Embedded Mode (Production)

Serves pre-built UI assets embedded in the binary:

```rust
use katana_explorer::{ExplorerLayer, ExplorerMode, ExplorerConfig};

// Simple embedded mode
let layer = ExplorerLayer::embedded("KATANA".to_string())?;

// Or with full config
let config = ExplorerConfig {
    mode: ExplorerMode::Embedded,
    chain_id: "KATANA".to_string(),
    security_headers: true,
    ..Default::default()
};
let layer = ExplorerLayer::new(config)?;
```

### 2. FileSystem Mode with Hot Reload (Development)

Serves UI files from disk with automatic reload on changes:

```rust
use std::path::PathBuf;

// Simple filesystem mode with hot reload
let layer = ExplorerLayer::filesystem(
    PathBuf::from("./ui/dist"),
    "KATANA_DEV".to_string(),
    true, // hot_reload enabled
)?;

// Or with full config
let config = ExplorerConfig {
    mode: ExplorerMode::FileSystem {
        ui_path: PathBuf::from("./ui/dist"),
        hot_reload: true,
    },
    chain_id: "KATANA_DEV".to_string(),
    cors_enabled: true,
    security_headers: false, // Disable for development
    ..Default::default()
};
```

### 3. Proxy Mode (Future)

Proxy requests to an external development server (planned feature):

```rust
use url::Url;

let layer = ExplorerLayer::proxy(
    Url::parse("http://localhost:3000")?,
    "KATANA".to_string(),
)?;
```

## Configuration

```rust
use katana_explorer::{ExplorerConfig, ExplorerMode};
use std::collections::HashMap;

let config = ExplorerConfig {
    // Serving mode
    mode: ExplorerMode::FileSystem { 
        ui_path: PathBuf::from("./ui/dist"),
        hot_reload: true,
    },
    
    // Chain configuration
    chain_id: "KATANA".to_string(),
    
    // Server configuration  
    path_prefix: "/explorer".to_string(), // URL prefix
    cors_enabled: true,                   // Enable CORS
    security_headers: true,               // Add security headers
    compression: false,                   // Enable compression (future)
    
    // Custom headers
    custom_headers: {
        let mut headers = HashMap::new();
        headers.insert("X-Custom-Header".to_string(), "value".to_string());
        headers
    },
    
    // UI environment variables
    ui_env: {
        let mut env = HashMap::new();
        env.insert("DEBUG".to_string(), serde_json::Value::Bool(true));
        env.insert("API_URL".to_string(), serde_json::Value::String("/api".to_string()));
        env
    },
};
```

## Integration with Tower Services

```rust
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;

let service = ServiceBuilder::new()
    .layer(CorsLayer::permissive())
    .layer(explorer_layer)
    .service_fn(your_main_service);
```

## Building the UI

### With Embedded UI (Production)

Enable the `embedded-ui` feature (enabled by default):

```toml
[dependencies]
katana-explorer = { version = "1.0", features = ["embedded-ui"] }
```

The build script will automatically:
1. Initialize the UI git submodule
2. Install dependencies with Bun
3. Build the UI assets
4. Embed them in the binary

### Without Embedded UI (Development/Custom)

Disable the `embedded-ui` feature:

```toml
[dependencies]
katana-explorer = { version = "1.0", default-features = false }
```

Then use `FileSystem` or `Proxy` mode to serve UI assets.

## Development Workflow

1. **Initial Setup**:
   ```bash
   git submodule update --init --recursive
   cd ui && bun install
   ```

2. **Development Mode** (with hot reload):
   ```rust
   let layer = ExplorerLayer::filesystem(
       PathBuf::from("./ui/dist"), 
       "KATANA_DEV".to_string(),
       true // Enable hot reload
   )?;
   ```

3. **Build UI** (when needed):
   ```bash
   cd ui && bun run build
   ```

4. **Start Katana** with filesystem mode - UI changes will be picked up automatically!

## Environment Variables

The explorer automatically injects environment variables into the UI:

### Default Variables
- `CHAIN_ID`: The configured chain ID
- `ENABLE_CONTROLLER`: Boolean flag for controller integration

### Custom Variables
Add custom variables through the `ui_env` config:

```rust
let mut env = HashMap::new();
env.insert("DEBUG".to_string(), serde_json::Value::Bool(true));
env.insert("API_ENDPOINT".to_string(), serde_json::Value::String("/api/v1".to_string()));

let config = ExplorerConfig {
    ui_env: env,
    // ... other config
};
```

These are accessible in the UI via `window.KATANA_CONFIG`:

```javascript
// In your UI code
const config = window.KATANA_CONFIG;
console.log('Chain ID:', config.CHAIN_ID);
console.log('Debug mode:', config.DEBUG);
```

## File Structure

```
crates/explorer/
├── src/lib.rs              # Main implementation
├── build.rs                # UI build script (feature-gated)
├── examples/
│   └── hot_reload_example.rs # Usage examples
├── ui/                      # UI submodule (git submodule)
│   ├── src/                 # UI source code
│   ├── dist/                # Built assets (generated)
│   └── package.json         # UI dependencies
└── README.md               # This file
```

## Performance & Caching

- **Embedded Mode**: Assets served directly from memory
- **FileSystem Mode**: Files cached in memory with mtime-based invalidation
- **Hot Reload**: Cache automatically cleared on file system changes
- **HTTP Headers**: Appropriate cache-control headers for different asset types

## Security

- **Content Security Policy**: Configurable CSP headers
- **CORS**: Optional CORS support for development
- **X-Frame-Options**: Prevents embedding in frames
- **Content-Type Sniffing**: Disabled for security

## Error Handling

- **Missing Assets**: Graceful fallback to embedded assets (if available)
- **File Errors**: Detailed logging and fallback behavior
- **Build Failures**: Non-fatal warnings when UI build fails

## Examples

See the `examples/` directory for complete usage examples:

- `hot_reload_example.rs`: Development workflow with hot reload

Run examples with:
```bash
cargo run --example hot_reload_example
```

## Troubleshooting

### UI Assets Not Found
- Ensure `ui/dist` directory exists and contains built files
- Check that git submodules are initialized
- Verify Bun is installed for building UI

### Hot Reload Not Working
- Ensure `hot_reload: true` in FileSystem mode
- Check file system permissions
- Verify the UI path exists and is readable

### Build Errors
- Install Bun: https://bun.sh
- Initialize submodules: `git submodule update --init --recursive`
- Build manually: `cd ui && bun run build`