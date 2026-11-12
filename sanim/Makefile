RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO = $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
ARGS ?=

.PHONY: check test fmt fmt-check bench frame-cache-bench state-cache-bench run build
check: fmt
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace --no-fail-fast
test:
	$(CARGO) test --workspace --no-fail-fast
fmt:
	uv run --no-project python ../packaging/format_workspace.py --manifest Cargo.toml -- $(CARGO)
fmt-check:
	uv run --no-project python ../packaging/format_workspace.py --manifest Cargo.toml --check -- $(CARGO)
bench:
	$(CARGO) bench -p sanim-lang --bench headless
frame-cache-bench:
	$(CARGO) run -p sanim-render --example frame_cache -- $(ARGS)
state-cache-bench:
	$(CARGO) run -p sanim-lang --example state_reload -- $(ARGS)
run:
	$(CARGO) run -p sanim-cli -- $(ARGS)
build:
	$(CARGO) build --workspace --release
