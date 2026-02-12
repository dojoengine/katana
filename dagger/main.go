// Katana CI pipeline — replicates core CI checks (fmt, clippy, build, test)
// so they can run identically locally via `dagger call` and in GitHub Actions.
package main

import (
	"context"
	"fmt"

	"katana-ci/internal/dagger"

	"golang.org/x/sync/errgroup"
)

type KatanaCi struct{}

// ----- helpers -----

// base returns a container pre-configured with the dev image, LLVM env vars,
// source mount, and cargo cache volumes.
func (m *KatanaCi) base(src *dagger.Directory) *dagger.Container {
	return dag.Container().
		From("ghcr.io/dojoengine/katana-dev:latest").
		WithEnvVariable("MLIR_SYS_190_PREFIX", "/usr/lib/llvm-19/").
		WithEnvVariable("LLVM_SYS_191_PREFIX", "/usr/lib/llvm-19/").
		WithEnvVariable("TABLEGEN_190_PREFIX", "/usr/lib/llvm-19/").
		WithEnvVariable("CARGO_TERM_COLOR", "always").
		WithMountedDirectory("/src", src).
		WithMountedCache("/root/.cargo/registry", dag.CacheVolume("cargo-registry")).
		WithMountedCache("/root/.cargo/git", dag.CacheVolume("cargo-git")).
		WithMountedCache("/src/target", dag.CacheVolume("cargo-target")).
		WithWorkdir("/src")
}

// ----- exported functions -----

// Fmt checks Rust formatting using the project's formatting script.
func (m *KatanaCi) Fmt(src *dagger.Directory) *dagger.Container {
	return dag.Container().
		From("ghcr.io/dojoengine/katana-dev:latest").
		WithMountedDirectory("/src", src).
		WithWorkdir("/src").
		WithExec([]string{"scripts/rust_fmt.sh", "--check"})
}

// GenerateTestArtifacts runs `make fixtures` and returns a directory containing
// the six fixture paths needed by downstream steps.
func (m *KatanaCi) GenerateTestArtifacts(src *dagger.Directory) *dagger.Directory {
	fixtures := dag.Container().
		From("ghcr.io/dojoengine/katana-dev:latest").
		WithMountedDirectory("/src", src).
		WithWorkdir("/src").
		// Need git for submodule operations; mark safe directory.
		WithExec([]string{"git", "config", "--global", "--add", "safe.directory", "/src"}).
		WithExec([]string{"git", "submodule", "update", "--init", "--recursive"}).
		WithExec([]string{"make", "fixtures"}).
		Directory("/src")

	// Return only the fixture paths to keep the artifact small.
	return dag.Directory().
		WithDirectory("tests/snos/snos/build", fixtures.Directory("tests/snos/snos/build")).
		WithDirectory("crates/contracts/build", fixtures.Directory("crates/contracts/build")).
		WithDirectory("tests/fixtures/db/spawn_and_move", fixtures.Directory("tests/fixtures/db/spawn_and_move")).
		WithDirectory("tests/fixtures/db/simple", fixtures.Directory("tests/fixtures/db/simple")).
		WithDirectory("tests/fixtures/db/1_6_0", fixtures.Directory("tests/fixtures/db/1_6_0")).
		WithDirectory("tests/fixtures/db/snos", fixtures.Directory("tests/fixtures/db/snos"))
}

// Clippy runs the project's clippy script with pre-built fixture artifacts
// overlaid onto the source tree.
func (m *KatanaCi) Clippy(src *dagger.Directory, fixtures *dagger.Directory) *dagger.Container {
	return m.base(src).
		WithDirectory("/src", fixtures).
		WithExec([]string{"./scripts/clippy.sh"})
}

// BuildKatanaBinary compiles the katana binary with all features and returns it.
func (m *KatanaCi) BuildKatanaBinary(src *dagger.Directory, fixtures *dagger.Directory) *dagger.File {
	return m.base(src).
		WithDirectory("/src", fixtures).
		WithExec([]string{"cargo", "build", "--bin", "katana", "--all-features"}).
		File("/src/target/debug/katana")
}

// Test runs the full nextest suite using the CI profile.
func (m *KatanaCi) Test(src *dagger.Directory, fixtures *dagger.Directory, binary *dagger.File) *dagger.Container {
	return m.base(src).
		WithDirectory("/src", fixtures).
		WithFile("/usr/local/bin/katana", binary, dagger.ContainerWithFileOpts{Permissions: 0o755}).
		WithEnvVariable("NEXTEST_PROFILE", "ci").
		WithExec([]string{
			"cargo", "nextest", "run",
			"--all-features",
			"--workspace",
			"--exclude", "snos-integration-test",
			"--exclude", "db-compat-test",
			"--build-jobs", "20",
		})
}

// All orchestrates the full CI pipeline: fmt → generate-test-artifacts → (clippy + build) → test.
func (m *KatanaCi) All(ctx context.Context, src *dagger.Directory) (string, error) {
	// 1. Format check (fast, no compilation)
	_, err := m.Fmt(src).Sync(ctx)
	if err != nil {
		return "", fmt.Errorf("fmt failed: %w", err)
	}

	// 2. Generate test fixtures
	fixtures := m.GenerateTestArtifacts(src)

	// 3. Clippy and build in parallel
	var binary *dagger.File
	g, ctx := errgroup.WithContext(ctx)

	g.Go(func() error {
		_, err := m.Clippy(src, fixtures).Sync(ctx)
		if err != nil {
			return fmt.Errorf("clippy failed: %w", err)
		}
		return nil
	})

	g.Go(func() error {
		b := m.BuildKatanaBinary(src, fixtures)
		// Force evaluation so we can capture the file reference.
		_, err := b.Size(ctx)
		if err != nil {
			return fmt.Errorf("build failed: %w", err)
		}
		binary = b
		return nil
	})

	if err := g.Wait(); err != nil {
		return "", err
	}

	// 4. Test (needs both clippy and build to have passed)
	_, err = m.Test(src, fixtures, binary).Sync(ctx)
	if err != nil {
		return "", fmt.Errorf("test failed: %w", err)
	}

	return "all checks passed", nil
}
