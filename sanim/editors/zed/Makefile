RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO = $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
CARGO_TARGET_DIR ?= target
export CARGO_TARGET_DIR
WASM_TARGET = wasm32-wasip2
PYTHON ?= python3

.PHONY: check fmt target build install install-check
check:
	$(CARGO) fmt --check
	$(CARGO) clippy --locked --all-targets -- -D warnings
	$(PYTHON) install.py --check

fmt:
	$(CARGO) fmt

target:
	$(RUSTUP) target add $(WASM_TARGET) --toolchain $(RUST_TOOLCHAIN)

build:
	CARGO_INCREMENTAL=0 RUSTC_WRAPPER= RUSTFLAGS= CARGO_ENCODED_RUSTFLAGS= $(CARGO) build --locked --release --target $(WASM_TARGET)
	cp "$(CARGO_TARGET_DIR)/$(WASM_TARGET)/release/shrimply_sanim_zed.wasm" extension.wasm
	$(PYTHON) install.py --check-wasm

install: build
	$(PYTHON) install.py

install-check: build
	$(PYTHON) -m unittest discover -s tests
