## katana-executor

### Cairo Native support

Cairo Native makes the execution of Sierra programs possible through native machine code. To use it, you must enable the `native` feature when using this crate as a dependency,

```toml
[dependencies]
katana-executor = { .., features = [ "native" ] }
```

and the following needs to be setup:

#### Dependencies
- Linux or macOS (aarch64 included) only for now
- LLVM 19 with MLIR: On debian you can use [apt.llvm.org](https://apt.llvm.org/), on macOS you can use brew

For Debian/Ubuntu:

```bash
sudo apt-get install llvm-19 llvm-19-dev llvm-19-runtime clang-19 clang-tools-19 lld-19 libpolly-19-dev libmlir-19-dev mlir-19-tools
```

You can automatically set environment variables by sourcing `cairo-native.env.sh`:
```bash
source cairo-native.env.sh
```

Alternatively, manually set them:

```bash
# For Debian/Ubuntu using the repository, the path will be /usr/lib/llvm-19
export MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
export LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
export TABLEGEN_190_PREFIX=/usr/lib/llvm-19
```

For macOS:

```console
brew install llvm@19
```

Source environment script:
```bash
source cairo-native.env.sh
```

Alternatively, manually set:
```bash
# If installed with brew
export LIBRARY_PATH=/opt/homebrew/lib
export MLIR_SYS_190_PREFIX="$(brew --prefix llvm@19)"
export LLVM_SYS_191_PREFIX="$(brew --prefix llvm@19)"
export TABLEGEN_190_PREFIX="$(brew --prefix llvm@19)"
```
