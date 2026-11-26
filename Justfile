# Task runner for Downmark local validation and packaging.

default:
    @just --list

# Format Rust code and check TOML formatting if taplo is installed
fmt:
    cargo fmt --all
    @if command -v taplo >/dev/null 2>&1; then taplo fmt; fi

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check
    @if command -v taplo >/dev/null 2>&1; then taplo fmt --check; fi

# Lint Rust code with Clippy across all targets
lint:
    cargo clippy --locked --all-targets -- -D warnings

# Verify default and no-default-features builds
check-features:
    cargo check --locked --all-targets
    cargo check --locked --no-default-features --all-targets

# Run tests (uses cargo-nextest if installed, falling back to cargo test)
test:
    @if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run --locked; else cargo test --locked; fi

# Run dependency security, license, and unused-crate checks
audit:
    cargo deny check
    cargo machete

# Check spelling with typos
typos:
    typos

# Lint GitHub Actions workflows and shell scripts
lint-infra:
    actionlint
    zizmor .github/workflows .github/dependabot.yml
    shellcheck packaging/linux/generate-cargo-sources.sh packaging/macos/bundle.sh packaging/macos/notarize.sh
    @if command -v shfmt >/dev/null 2>&1; then shfmt -d -i 2 -ci packaging/linux/generate-cargo-sources.sh packaging/macos/bundle.sh packaging/macos/notarize.sh; fi

# Validate Linux desktop and AppStream metadata
validate-linux:
    desktop-file-validate packaging/linux/io.github.Vidhan31.Downmark.desktop
    appstreamcli validate --no-net --pedantic packaging/linux/io.github.Vidhan31.Downmark.metainfo.xml

# Regenerate offline Cargo sources for Flatpak builds
flatpak-sources:
    ./packaging/linux/generate-cargo-sources.sh

# Run full local CI checks
ci: fmt-check lint check-features test typos

# Clean Cargo and packaging build outputs
clean:
    cargo clean
    rm -rf dist
