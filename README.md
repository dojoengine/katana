# Katana

## Table of Contents

- [Development Setup](#development-setup)
- [Cairo Native](#cairo-native)
- [Explorer](#explorer)
- [Testing](#testing)

## Development Setup

### Rust

The project is built with the Rust programming language. You'll need to have Rust and Cargo (the Rust package manager) installed first in order to start developing.
Follow the installation steps here: https://www.rust-lang.org/tools/install

### Scarb

Scarb is the Cairo package manager required for building the feature contracts in [`katana-contracts`](https://github.com/dojoengine/katana/tree/main/crates/contracts). The project requires a specific version of Scarb (2.8.4) to ensure compatibility.

To install the required version of `scarb`:

```bash
make install-scarb
```

This command will check if the correct version is already installed and only install it if necessary. For further information on `scarb`, check its [documentations page](https://docs.swmansion.com/scarb/docs.html).

### LLVM Dependencies

For Cairo native support, you'll need to install LLVM dependencies:

#### For macOS:
```bash
make native-deps-macos
```

#### For Linux:
```bash
make native-deps-linux
```

After installing LLVM, you need to make sure the required environment variables are set for your current shell:

```bash
source scripts/cairo-native.env.sh
```

### Bun

When building the project, you may need to build the [Explorer](#explorer) web application.
For that, you need to have [Bun](https://bun.sh/docs/installation) installed.

The actual building flow will be handled automatically by Cargo, but it can also be built manually:

```bash
make build-explorer
```

## Cairo Native

Katana supports Cairo Native execution, which significantly improves the performance of Starknet contract execution by compiling Cairo contracts into optimized machine code.

Cairo Native uses a multi-stage compilation process (Sierra → MLIR → LLVM → Native Executables) to generate fast, efficient binaries. This reduces the overhead of virtual machine emulation and allows Katana to process transactions at much higher speeds. Check out the [`cairo_native`](https://github.com/lambdaclass/cairo_native) repository to learn more.

To build the Katana binary from source with Cairo Native support, make sure to enable the `native` Cargo feature:

> _NOTE: Ensure you have configured the necessary [LLVM dependencies](#llvm-dependencies) before proceeding_.

```bash
cargo build --bin katana --features native
```

Cairo Native is disabled by default but can be enabled at runtime by specifying the `--enable-native-compilation` flag.

## Explorer

Katana includes a built-in, developer-focused, and stateless block explorer called **Explorer**. It is bundled with the Katana binary and can be enabled using the `--explorer` flag:

```bash
katana --explorer
```

Once enabled, the Explorer web application will be served at the `/explorer` path relative to the Katana RPC server endpoint. For example, if the RPC server is running at `http://localhost:5050`, the Explorer will be accessible at `http://localhost:5050/explorer`.

This makes it easy for developers to inspect blocks, transactions, and other on-chain data in a lightweight, self-hosted interface. The Explorer is designed to be fast, minimal, and fully integrated into the Katana workflow without requiring additional setup. To learn more about the Explorer or contribute to its development, visit the [repository](https://github.com/cartridge-gg/explorer/).

## Testing

We recommend using `cargo nextest` for running the tests. Nextest is a next-generation test runner for Rust that provides better performance and user experience compared to `cargo test`. For more information on `cargo-nextest`, including installation instructions, please refer to the [official documentation](https://nexte.st/).

### Setting Up the Test Environment

Before running tests, you need to set up the test environment by generating all necessary artifacts:

```bash
make fixtures
```

Once setup is complete, you can run the tests using:

```bash
cargo nextest run
```
