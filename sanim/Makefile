RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO = $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
ARGS ?=

.PHONY: media-check media-bench
media-check:
	$(CARGO) test -p sanim-core -p sanim-data -p sanim-lang -p sanim-render --test media
media-bench:
	$(CARGO) test $(ARGS) -p sanim-data --test media media_benchmark -- --ignored --nocapture
LSP_INSTALL_DIR ?= $(HOME)/.local/bin

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
.PHONY: reflection-schema-bench
reflection-schema-bench:
	$(CARGO) test $(ARGS) -p sanim-core --test reflection canonical_registry_lookup_benchmark -- --ignored --nocapture
runtime-hot-bench:
	$(CARGO) run -p sanim-runtime --example hot_reload -- $(ARGS)
run:
	$(CARGO) run -p sanim-cli -- $(ARGS)
build:
	$(CARGO) build --workspace --exclude sanim-lsp --release

.PHONY: lsp-build lsp-check lsp-bench lsp-utility-bench lsp-install
lsp-install: lsp-build
	mkdir -p "$(LSP_INSTALL_DIR)"
	install -m 755 "$(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)/release/sanim-lsp" "$(LSP_INSTALL_DIR)/.sanim-lsp.new"
	mv -f "$(LSP_INSTALL_DIR)/.sanim-lsp.new" "$(LSP_INSTALL_DIR)/sanim-lsp"
lsp-bench:
	$(CARGO) test -p sanim-lsp --test protocol incremental_analysis_benchmark -- --ignored --nocapture
lsp-utility-bench:
	$(CARGO) test -p sanim-lsp --test protocol markup_utility_completion_benchmark -- --ignored --nocapture
lsp-build:
	$(CARGO) build -p sanim-lsp --release
lsp-check:
	$(CARGO) clippy -p sanim-lsp --all-targets -- -D warnings
	$(CARGO) test -p sanim-lsp --no-fail-fast

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

.PHONY: demand-check demand-bench render-lazy-check render-lazy-bench dev-cli-check
demand-check:
	$(CARGO) test -p sanim-lang --test demand
demand-bench:
	$(CARGO) run -p sanim-lang --example demand
render-lazy-check:
	$(CARGO) clippy -p sanim-render --all-targets -- -D warnings
	$(CARGO) test -p sanim-render --no-fail-fast
render-lazy-bench:
	$(CARGO) run -p sanim-render --example lazy_geometry
dev-cli-check:
	$(CARGO) test -p sanim-cli --no-fail-fast

.PHONY: format-check format-differential format-bench strings-check lint-check
.PHONY: plot-check plot-bench
.PHONY: dataframe-check dataframe-bench
dataframe-check:
	$(CARGO) clippy -p sanim-core -p sanim-data -p sanim-lang --all-targets -- -D warnings
	$(CARGO) test -p sanim-data --test dataframe
	$(CARGO) test -p sanim-lang --test dataframe
dataframe-bench:
	$(CARGO) test $(ARGS) -p sanim-data --test dataframe dataframe_benchmark -- --ignored --nocapture
	$(CARGO) test $(ARGS) -p sanim-lang --test dataframe dataframe_hot_reload_benchmark -- --ignored --nocapture
plot-check:
	$(CARGO) test -p sanim-core --test plots
	$(CARGO) test -p sanim-lang --test plots
	$(CARGO) test -p sanim-render --test plots
plot-bench:
	$(CARGO) run --release -p sanim-render --example plots
format-bench:
	$(CARGO) test -p sanim-core --test format formatter_microbenchmark -- --ignored --nocapture
strings-check:
	$(CARGO) test -p sanim-lang --test strings
lint-check:
	$(CARGO) test -p sanim-lang --test lint
	$(CARGO) test -p sanim-data --test text
format-check:
	$(CARGO) test -p sanim-core --test format
	$(CARGO) test -p sanim-lang --test format
format-differential:
	$(CARGO) test -p sanim-core --test format python_differential_reference_formats -- --ignored --nocapture

.PHONY: source-format-check source-format-bench markup-check markup-bench sql-template-check sql-template-bench
sql-template-check:
	$(CARGO) clippy -p sanim-core -p sanim-data -p sanim-lang --all-targets -- -D warnings
	$(CARGO) test -p sanim-core --test templates
	$(CARGO) test -p sanim-lang --test sql_templates
sql-template-bench:
	$(CARGO) test $(ARGS) -p sanim-core --test templates template_benchmark -- --ignored --nocapture
.PHONY: signal-function-check signal-function-bench
signal-function-check:
	$(CARGO) clippy -p sanim-lang -p sanim-lsp --all-targets -- -D warnings
	$(CARGO) test -p sanim-lang --test signal_functions
	$(CARGO) test -p sanim-lsp
signal-function-bench:
	$(CARGO) test --release -p sanim-lang --test signal_functions signal_function_benchmark -- --ignored --nocapture
source-format-check:
	$(CARGO) clippy -p sanim-lang --all-targets -- -D warnings
	$(CARGO) test -p sanim-lang --test source_formatter
source-format-bench:
	$(CARGO) test -p sanim-lang --test source_formatter source_formatter_benchmark -- --ignored --nocapture
markup-check:
	$(CARGO) clippy -p sanim-lang -p sanim-render --all-targets -- -D warnings
	$(CARGO) test -p sanim-lang --test markup
	$(CARGO) test -p sanim-render --test markup
markup-bench:
	$(CARGO) run --release -p sanim-render --example markup

.PHONY: picking-check picking-bench
.PHONY: provenance-check provenance-bench
provenance-check:
	$(CARGO) test -p sanim-lang --test provenance
.PHONY: code-view-render-check
code-view-render-check:
	$(CARGO) test -p sanim-render --test code_view
provenance-bench:
	$(CARGO) test --release -p sanim-lang --test provenance provenance_lookup_and_reload_benchmark -- --ignored --nocapture
picking-check:
	$(CARGO) test -p sanim-render --test picking
picking-bench:
	$(CARGO) test --release -p sanim-render --test picking picking_benchmark -- --ignored --nocapture

.PHONY: code-view-check code-view-test code-view-bench
code-view-check:
	$(CARGO) clippy -p sanim-data -p sanim-lang -p sanim-render -p sanim-lsp --all-targets -- -D warnings
	$(MAKE) code-view-test
code-view-test:
	$(CARGO) test -p sanim-core -p sanim-data -p sanim-lang -p sanim-render --test code
code-view-bench:
	$(CARGO) test --release -p sanim-data -p sanim-lang -p sanim-render --test code code_view_benchmark -- --ignored --nocapture

.PHONY: runtime-watch-check
.PHONY: hot-reload-io-check hot-reload-memory-bench
hot-reload-io-check:
	$(CARGO) test -p sanim-core --test reactive reactive_edits_invalidate_only_reachable_elements_after_their_birth
	$(CARGO) test -p sanim-core --test list_views changed_signals_on_future_list_items_invalidate_the_mounted_view_only
	$(CARGO) test -p sanim-lang --test hot_reload_io
	$(CARGO) test -p sanim-runtime --lib watcher::tests
	$(CARGO) test -p sanim-runtime --test watching scalar_hot_edits_release_old_revisions_and_preserve_glyphs_and_prefix_frames
hot-reload-memory-bench:
	$(CARGO) test -p sanim-runtime --test watching scalar_hot_reload_memory_plateau_benchmark -- --ignored --nocapture
.PHONY: native-cache-check native-cache-bench
native-cache-check:
	$(CARGO) test -p sanim-render --test native_cache
native-cache-bench:
	$(CARGO) test --release -p sanim-render --test native_cache native_endpoint_benchmark -- --ignored --nocapture
.PHONY: reactive-label-check reactive-label-bench
.PHONY: code-transition-check code-transition-bench
.PHONY: font-budget-check
font-budget-check:
	$(CARGO) test -p sanim-core --test reactive font_
	$(CARGO) test -p sanim-lang --test font_budget
.PHONY: code-patch-check
code-patch-check:
	$(CARGO) test -p sanim-render --test code_patches
code-transition-check:
	$(CARGO) test -p sanim-data --test code_transitions
	$(CARGO) test -p sanim-data cursor_boundaries_keep_zwj_flags_and_combining_sequences_whole --lib
	$(CARGO) test -p sanim-data ordered_paths_ --lib
code-transition-bench:
	$(CARGO) test --release -p sanim-data --test code_transitions transition_benchmark -- --ignored --nocapture
reactive-label-check:
	$(CARGO) test -p sanim-lang --test reactive_labels
	$(CARGO) test -p sanim-render --test reactive_labels
reactive-label-bench:
	$(CARGO) test --release -p sanim-render --test reactive_labels reactive_label_benchmark -- --ignored --nocapture
runtime-watch-check:
	$(CARGO) test -p sanim-runtime --test watching

.PHONY: scrolling-check scrolling-test scrolling-bench
scrolling-check:
	$(CARGO) clippy -p sanim-core -p sanim-lang -p sanim-render --all-targets -- -D warnings
	$(MAKE) scrolling-test
scrolling-test:
	$(CARGO) test -p sanim-core --test scrollbar_geometry
	$(CARGO) test -p sanim-lang -p sanim-render --test scrolling
scrolling-bench:
	$(CARGO) test --release -p sanim-render --test scrolling scroll_benchmark -- --ignored --nocapture

.PHONY: literal-type-check
literal-type-check:
	$(CARGO) test -p sanim-lang --test literal_types
	$(CARGO) clippy -p sanim-core -p sanim-data -p sanim-lang --all-targets -- -D warnings

.PHONY: literal-validation-bench
literal-validation-bench:
	$(CARGO) test --release -p sanim-core --test literal_validation_benchmark -- --ignored --nocapture

.PHONY: parsed-cache-check
parsed-cache-check:
	$(CARGO) test -p sanim-lang --lib cache::parsed::tests

.PHONY: reactive-watch-check reactive-diff-bench
reactive-watch-check:
	$(CARGO) test -p sanim-core --test reactive changed_binding_visitor
	$(CARGO) test -p sanim-core --test list_views changed_signals_on_future
	$(CARGO) test -p sanim-lang --test reload
reactive-diff-bench:
	$(CARGO) test $(ARGS) -p sanim-core --test reactive reactive_binding_diff_benchmark -- --ignored --nocapture

.PHONY: integration-fixtures-check
integration-fixtures-check:
	$(CARGO) test -p sanim-core --test timed_values generic_state_never_weakens_property_expression_types_or_validation
	$(CARGO) test -p sanim-lang --test generics type_aliases_support_defaults_and_constraints
	$(CARGO) test -p sanim-lang --test scrolling overflow_errors_explain_required_container_and_dimensions

.PHONY: box-migration-check
box-migration-check:
	$(CARGO) test -p sanim-core --test reactive box_dimensions
	$(CARGO) test -p sanim-lang --test animation --test charts --test plots --test spring --no-fail-fast
	$(CARGO) test -p sanim-render --test frame_cache --test list_views --test picking --test plots --test rendering --no-fail-fast

.PHONY: perf-reload-check
perf-reload-check:
	$(CARGO) test -p sanim-runtime --lib watcher::tests::native_session_suffix_edits_preserve_prehashed_resources_and_execution_work

.PHONY: perf-reload-bench
perf-reload-bench:
	$(CARGO) test --release -p sanim-runtime --lib watcher::tests::cached_import_size_does_not_scale_hot_reload_like_rehashing -- --ignored --nocapture

.PHONY: performance-check
performance-check: perf-reload-check
	$(CARGO) test -p sanim-render --test performance
	$(CARGO) test -p sanim-lang --lib cache::parsed::tests
	$(CARGO) test -p sanim-runtime --test watching scalar_hot_edits_release_old_revisions_and_preserve_glyphs_and_prefix_frames

.PHONY: text-wrapping-check
text-wrapping-check:
	$(CARGO) clippy -p sanim-core -p sanim-data -p sanim-lang -p sanim-render --all-targets -- -D warnings
	$(CARGO) test -p sanim-core -p sanim-render --test text_wrapping
	$(CARGO) test -p sanim-data --lib code::view::

.PHONY: text-wrapping-bench
text-wrapping-bench:
	$(CARGO) test --release -p sanim-data --lib width_reflow_benchmark -- --ignored --nocapture

.PHONY: text-cow-check
text-cow-check:
	$(CARGO) test -p sanim-core --test reactive shared_geometry_text_and_language_values_charge_one_weakly_guarded_payload
	$(CARGO) test -p sanim-lang --test reactive_labels text_constructor_getter_and_setter_share_language_string_allocations

.PHONY: text-wrapping-integration-bench
text-wrapping-integration-bench:
	$(CARGO) test --release -p sanim-render --test text_wrapping codeview_frame_pipeline_benchmark -- --ignored --nocapture
