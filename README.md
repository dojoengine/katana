# Katana

Katana is a Rust-based blockchain project with a focus on Starknet compatibility.

## Table of Contents

- [Development Setup](#development-setup)
- [Testing](#testing)

## Development Setup

### Rust

The project is built with Rust. You'll need to have Rust and Cargo installed first in order to start developing.
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

## Testing

### Setting Up the Test Environment

Before running tests, you need to set up the test environment by generating all necessary artifacts. The project includes a convenient `make` target that handles this process for you:

```bash
make test-setup
```

Once setup is complete, you can run the tests using:

```bash
cargo test
```
