RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO = $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
ARGS ?=

.PHONY: core-data-check list-bench
core-data-check:
	$(CARGO) clippy -p sanim-core -p sanim-data --all-targets -- -D warnings
	$(CARGO) test -p sanim-core -p sanim-data --no-fail-fast
list-bench:
	$(CARGO) test -p sanim-core -p sanim-data list_benchmark -- --ignored --nocapture

.PHONY: check test fmt fmt-check bench frame-cache-bench state-cache-bench runtime-hot-bench reflection-bench run build
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
reflection-bench:
	$(CARGO) run -p sanim-lang --example reflection_reload -- $(ARGS)
runtime-hot-bench:
	$(CARGO) run -p sanim-runtime --example hot_reload -- $(ARGS)
run:
	$(CARGO) run -p sanim-cli -- $(ARGS)
build:
	$(CARGO) build --workspace --release

.PHONY: reactive-bench
reactive-bench:
	$(CARGO) test -p sanim-core retained_frame_benchmark -- --ignored --nocapture

.PHONY: state-reactive-bench
state-reactive-bench:
	$(CARGO) run -p sanim-lang --example reactive_state -- $(ARGS)

.PHONY: collection-storage-bench
collection-storage-bench:
	$(CARGO) test -p sanim-core -p sanim-data collection_storage_benchmark -- --ignored --nocapture

.PHONY: collection-language-bench
collection-language-bench:
	$(CARGO) run -p sanim-lang --example collection_edits

.PHONY: list-view-check list-view-bench
list-view-check:
	$(CARGO) test -p sanim-core -p sanim-render --test list_views
list-view-bench:
	$(CARGO) test -p sanim-core -p sanim-render --test list_views retained_list_ -- --ignored --nocapture

.PHONY: pop-bench
pop-bench:
	$(CARGO) test -p sanim-lang --test list_edits pop_benchmark -- --ignored --nocapture

.PHONY: font-check font-bench
font-check:
	$(CARGO) test -p sanim-data --test fonts
	$(CARGO) test -p sanim-lang --test font_families
	$(CARGO) test -p sanim-lang font_ --lib
font-bench:
	$(CARGO) test -p sanim-data --test fonts native_family_shaping_benchmark -- --ignored --nocapture

.PHONY: animation-call-check
animation-call-check:
	$(CARGO) test -p sanim-lang --tests --no-fail-fast

.PHONY: spring-bench
spring-bench:
	$(CARGO) test -p sanim-core --test spring spring_sampling_benchmark -- --ignored --nocapture
