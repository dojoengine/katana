# Katana

## Table of Contents

- [Development Setup](#development-setup)
- [Cairo Native](#cairo-native)
- [Testing](#testing)

## Development Setup

### Rust

The project is built with the Rust programming language. You'll need to have Rust and Cargo (the Rust package manager) installed first in order to start developing.
Follow the installation steps here: https://www.rust-lang.org/tools/install

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

### Bun (for Explorer)

When building the project, you may need to build the Explorer application. For that, you need to have [Bun](https://bun.sh/docs/installation) installed.

Building the Explorer application will be handled automatically by Cargo, but it can also be built manually:

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

```bash
Environment options:
      --enable-native-compilation
          Enable cairo-native compilation for improved performance
```

## Testing

We recommend using `cargo nextest` for running the tests. Nextest is a next-generation test runner for Rust that provides better performance and user experience compared to `cargo test`. For more information on `cargo-nextest`, including installation instructions, please refer to the [official documentation](https://nexte.st/).

### Setting Up the Test Environment

Before running tests, you need to set up the test environment by generating all necessary artifacts:

```bash
make test-artifacts
```

Once setup is complete, you can run the tests using:

```bash
cargo nextest run
```
