CARGO ?= cargo
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
	rustup target add $(WASM_TARGET)

build:
	CARGO_INCREMENTAL=0 RUSTC_WRAPPER= RUSTFLAGS= CARGO_ENCODED_RUSTFLAGS= $(CARGO) build --locked --release --target $(WASM_TARGET)
	cp "$(CARGO_TARGET_DIR)/$(WASM_TARGET)/release/shrimply_sanim_zed.wasm" extension.wasm
	$(PYTHON) install.py --check-wasm

install: build
	$(PYTHON) install.py

install-check: build
	$(PYTHON) -m unittest discover -s tests
