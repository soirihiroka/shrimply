RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO = $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
ARGS ?=

.PHONY: check test fmt bench run build
check: fmt
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace --no-fail-fast
test:
	$(CARGO) test --workspace --no-fail-fast
fmt:
	$(CARGO) fmt --all
bench:
	$(CARGO) bench -p sanim-lang --bench headless
run:
	$(CARGO) run -p sanim-cli -- $(ARGS)
build:
	$(CARGO) build --workspace --release
