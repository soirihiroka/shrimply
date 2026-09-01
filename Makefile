RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO := $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
CUDA_HOME ?= /usr/local/cuda
CUDA_TOOLKIT_PATH ?= $(CUDA_HOME)
CUDA_OXIDE_TARGET ?= sm_86
CUDA_OXIDE_DEBUG ?= off
SOURCE_LINE_LIMIT ?= 2000
SLANG_SOURCE_DIR ?= $(CURDIR)/external/slang
SLANG_BUILD_DIR ?= $(SLANG_SOURCE_DIR)/build
OPTIX_ROOT ?= $(CURDIR)/external/optix-dev
DNF ?= sudo dnf
PACMAN ?= sudo pacman -S --needed
INSTALL ?= install
PKG_CONFIG ?= /usr/bin/pkg-config
PKG_CONFIG_PATH ?= /usr/lib64/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig
QT_QMAKE ?= qmake6
BUILD_ENV := CUDA_HOME=$(CUDA_HOME) CUDA_TOOLKIT_PATH=$(CUDA_TOOLKIT_PATH) PATH=$(CUDA_HOME)/bin:$(PATH) PKG_CONFIG=$(PKG_CONFIG) PKG_CONFIG_PATH=$(PKG_CONFIG_PATH) SLANG_SOURCE_DIR=$(SLANG_SOURCE_DIR) SLANG_BUILD_DIR=$(SLANG_BUILD_DIR) OPTIX_ROOT=$(OPTIX_ROOT)
RUST_LIBDIR := $(shell $(RUSTUP) run $(RUST_TOOLCHAIN) rustc --print target-libdir)
DEV_RUSTFLAGS ?= -C prefer-dynamic -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)
DEV_BUILD_ENV := $(BUILD_ENV) RUSTFLAGS="$(DEV_RUSTFLAGS)"
OXIDE_ENV := $(BUILD_ENV) CUDA_OXIDE_TARGET=$(CUDA_OXIDE_TARGET) CUDA_OXIDE_DEBUG=$(CUDA_OXIDE_DEBUG)
CUDA_ARTIFACT_DIR := $(CURDIR)/.oxide-artifacts/cuda/$(CUDA_OXIDE_TARGET)
CUDA_BUILD_TARGET := $(CURDIR)/target/cuda-oxide
PREVIEW_CUBIN := $(CUDA_ARTIFACT_DIR)/preview.cubin
ANIME4K_CUBIN := $(CUDA_ARTIFACT_DIR)/anime4k.cubin
MODIFIERS_CUBIN := $(CUDA_ARTIFACT_DIR)/modifiers.cubin
MODIFIERS_BLUR_CUBIN := $(CUDA_ARTIFACT_DIR)/modifiers-blur.cubin
MODIFIERS_GEOMETRY_CUBIN := $(CUDA_ARTIFACT_DIR)/modifiers-geometry.cubin
MODIFIERS_MATTE_CUBIN := $(CUDA_ARTIFACT_DIR)/modifiers-matte.cubin
STABILIZATION_CUBIN := $(CUDA_ARTIFACT_DIR)/stabilization.cubin
EXPORT_CUBIN := $(CUDA_ARTIFACT_DIR)/export.cubin
CUDA_CUBINS := $(PREVIEW_CUBIN) $(ANIME4K_CUBIN) $(MODIFIERS_CUBIN) $(MODIFIERS_BLUR_CUBIN) $(MODIFIERS_GEOMETRY_CUBIN) $(MODIFIERS_MATTE_CUBIN) $(STABILIZATION_CUBIN) $(EXPORT_CUBIN)
CUDA_COLOR_SOURCES := crates/math/color/src/lib.rs crates/math/color/src/adw.rs crates/math/color/src/blend.rs crates/math/color/Cargo.toml
CUDA_GEOMETRY_SOURCES := $(filter-out crates/math/geometry/src/skia.rs,$(shell find crates/math/geometry/src -type f)) crates/math/geometry/Cargo.toml
CUDA_SHARED_SOURCES := $(shell find crates/render-core/src -type f) crates/render-core/Cargo.toml $(CUDA_COLOR_SOURCES) $(CUDA_GEOMETRY_SOURCES)
CUDA_PREVIEW_SOURCES := $(shell find crates/cuda/preview/src -type f) crates/cuda/preview/Cargo.toml crates/cuda/preview/Cargo.lock
CUDA_ANIME4K_SOURCES := $(shell find crates/cuda/anime4k/src -type f) crates/video/anime4k/src/types.rs crates/cuda/anime4k/Cargo.toml crates/cuda/anime4k/Cargo.lock $(CUDA_COLOR_SOURCES)
CUDA_MODIFIER_SOURCES := $(shell find crates/cuda/modifiers/src -type f) crates/cuda/modifiers/Cargo.toml crates/cuda/modifiers/Cargo.lock
CUDA_MODIFIER_BLUR_SOURCES := $(shell find crates/cuda/modifiers-blur/src -type f) crates/cuda/modifiers-blur/Cargo.toml crates/cuda/modifiers-blur/Cargo.lock
CUDA_MODIFIER_GEOMETRY_SOURCES := $(shell find crates/cuda/modifiers-geometry/src -type f) crates/cuda/modifiers-geometry/Cargo.toml crates/cuda/modifiers-geometry/Cargo.lock
CUDA_MODIFIER_MATTE_SOURCES := $(shell find crates/cuda/modifiers-matte/src -type f) crates/cuda/modifiers-matte/Cargo.toml crates/cuda/modifiers-matte/Cargo.lock
CUDA_STABILIZATION_SOURCES := $(shell find crates/cuda/stabilization/src -type f) crates/cuda/stabilization/Cargo.toml crates/cuda/stabilization/Cargo.lock
CUDA_EXPORT_SOURCES := $(shell find crates/cuda/export/src -type f) crates/cuda/export/Cargo.toml crates/cuda/export/Cargo.lock $(CUDA_COLOR_SOURCES)
CUDA_LINK_SOURCES := $(shell find crates/cuda/link/src -type f) crates/cuda/link/Cargo.toml crates/cuda/link/Cargo.lock

APP_NAME := Shrimply
BIN_NAME := shrimply
EDITOR_BIN_NAME := shrimply-editor
EDITOR_PACKAGE := shrimply-editor-ui
QT_EDITOR_PACKAGE := shrimply-editor-qt-ui
LAUNCHER_PACKAGE := shrimply-launcher-ui
QT_LAUNCHER_PACKAGE := shrimply-launcher-qt-ui
GTK_COMPONENTS_PACKAGE := shrimply-gtk-components
QT_COMPONENTS_PACKAGE := shrimply-qt-components
GTK_COMPONENTS_DEMO_PACKAGE := shrimply-gtk-components-demo
QT_COMPONENTS_DEMO_PACKAGE := shrimply-qt-components-demo
QT_BIN_NAME := shrimply-qt
MCP_PACKAGE := shrimply-mcp
MCP_BIN_NAME := shrimply-mcp
MCP_SERVER_NAME ?= shrimply
CODEX ?= codex
AGY ?= agy
RUST_LOG ?= info,shrimply=debug,shrimply_editor=debug,shrimply_launcher=debug,shrimply_launcher_qt_ui=debug,shrimply_timeline_ui=debug
DEV_LOG ?= target/$(BIN_NAME)-dev.log
QT_DEV_LOG ?= target/$(QT_BIN_NAME)-dev.log
CRASH_CORE ?= target/$(EDITOR_BIN_NAME).core
CRASH_STACK ?= target/$(EDITOR_BIN_NAME).stack
CRASH_PROFILE ?= debug
CRASH_SINCE ?= -1 day

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
APPLICATIONSDIR ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/apps
DESKTOP_FILE := assets/dev.shrimply.Shrimply.desktop
QT_DESKTOP_FILE := assets/dev.shrimply.Shrimply.Qt.desktop
APP_ICON := assets/icons/dev.shrimply.Shrimply.svg
RHUBARB_LIBRARY := target/release/libshrimply-rhubarb.so
RHUBARB_MODEL_SOURCE := external/rhubarb-lip-sync/rhubarb/lib/pocketsphinx-rev13216/model/en-us
RHUBARB_ACOUSTIC_MODEL_SOURCE := external/rhubarb-lip-sync/rhubarb/lib/cmusphinx-en-us-5.2
RHUBARB_RESOURCE_DIR := $(DATADIR)/shrimply/rhubarb/sphinx

FEDORA_PACKAGES := \
	rust \
	cargo \
	boost-devel \
	clang \
	clang-devel \
	cmake \
	gcc-c++ \
	lld \
	ninja-build \
	opencv-devel \
	openssl-devel \
	pkgconf-pkg-config \
	gobject-introspection-devel \
	ffmpeg-devel \
	rubberband-devel \
	alsa-lib-devel \
	gtk4-devel \
	libadwaita-devel \
	pipewire-devel \
	libglvnd-devel \
	gtksourceview5-devel \
	vte291-gtk4-devel \
	poppler-glib-devel \
	freetype-devel

ARCH_PACKAGES := \
	base-devel \
	rustup \
	boost \
	clang \
	cmake \
	ninja \
	opencv \
	openssl \
	pkgconf \
	gobject-introspection \
	ffmpeg \
	rubberband \
	alsa-lib \
	gtk4 \
	libadwaita \
	pipewire \
	libglvnd \
	gtksourceview5 \
	vte4 \
	poppler-glib \
	freetype2 \
	lld \
	cuda

.PHONY: native-deps qt-native-deps qt-desktop-file cuda-target-check cuda-artifacts dev qt-build dev-qt dev-server docs docs-check run run-qt build release check components-check gtk-components-showcase qt-components-showcase server-python-check manim manim-python-check manim-parameter-check cargo-check fmt fmt-check lint test frame-rate-test video-lifecycle-test transparent-fill-frame-range-test transparent-fill-decoder-test transparent-fill-kernel-test transparent-fill-compositor-test transparent-fill-playback-test transparent-fill-e2e-fixture transparent-fill-e2e-test decode-ahead-benchmark paint-interpolation-test crash-report oxide-doctor oxide-setup clean-dev clean deps-fedora deps-arch oxide-setup-arch install install-codex-mcp-dev install-agy-mcp-dev uninstall

native-deps:
	@$(PKG_CONFIG) --exists rubberband || { echo "Missing Rubber Band development files (pkg-config: rubberband)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists libpipewire-0.3 || { echo "Missing PipeWire development files (pkg-config: libpipewire-0.3)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists poppler-glib || { echo "Missing Poppler GLib development files (pkg-config: poppler-glib)" >&2; exit 1; }

qt-native-deps:
	@command -v $(QT_QMAKE) >/dev/null 2>&1 || { echo "Missing Qt 6 qmake ($(QT_QMAKE))" >&2; exit 1; }
	@version="$$($(QT_QMAKE) -query QT_VERSION)"; case "$$version" in 6.*) echo "Using Qt $$version via $(QT_QMAKE)" ;; *) echo "$(QT_QMAKE) selected unsupported Qt $$version; Qt 6 is required" >&2; exit 1 ;; esac
	@$(PKG_CONFIG) --exists Qt6Core Qt6Gui Qt6Qml Qt6Quick Qt6QuickControls2 Qt6OpenGL || { echo "Missing Qt 6 Quick/OpenGL development files" >&2; exit 1; }

cuda-target-check:
	@test "$(CUDA_OXIDE_TARGET)" = sm_86 || { echo "CUDA_OXIDE_TARGET=$(CUDA_OXIDE_TARGET) is unsupported: host binaries embed sm_86 CUDA artifacts" >&2; exit 1; }

cuda-artifacts: cuda-target-check $(CUDA_CUBINS)

$(CUDA_CUBINS): | cuda-target-check

# Keep these dependencies limited to device and linker inputs. Depending on the
# root Cargo.lock or host sources makes ordinary GUI edits rebuild every cubin.
$(PREVIEW_CUBIN): $(CUDA_PREVIEW_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/preview && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_preview --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/preview.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/preview.ltoir "$$tmp" preview $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(ANIME4K_CUBIN): $(CUDA_ANIME4K_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/anime4k && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_anime4k --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/anime4k.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/anime4k.ltoir "$$tmp" anime4k $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(MODIFIERS_CUBIN): $(CUDA_MODIFIER_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/modifiers && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_modifiers --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/modifiers.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/modifiers.ltoir "$$tmp" modifiers $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(MODIFIERS_BLUR_CUBIN): $(CUDA_MODIFIER_BLUR_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/modifiers-blur && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_modifiers_blur --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/modifiers-blur.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/modifiers-blur.ltoir "$$tmp" modifiers-blur $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(MODIFIERS_GEOMETRY_CUBIN): $(CUDA_MODIFIER_GEOMETRY_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/modifiers-geometry && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_modifiers_geometry --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/modifiers-geometry.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/modifiers-geometry.ltoir "$$tmp" modifiers-geometry $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(MODIFIERS_MATTE_CUBIN): $(CUDA_MODIFIER_MATTE_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/modifiers-matte && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_modifiers_matte --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/modifiers-matte.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/modifiers-matte.ltoir "$$tmp" modifiers-matte $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(STABILIZATION_CUBIN): $(CUDA_STABILIZATION_SOURCES) $(CUDA_SHARED_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/stabilization && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_stabilization --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/stabilization.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/stabilization.ltoir "$$tmp" stabilization $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

$(EXPORT_CUBIN): $(CUDA_EXPORT_SOURCES) $(CUDA_LINK_SOURCES)
	@mkdir -p $(CUDA_ARTIFACT_DIR)
	cd crates/cuda/export && $(OXIDE_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) oxide emit-ltoir shrimply_cuda_export --arch $(CUDA_OXIDE_TARGET) -o $(CUDA_ARTIFACT_DIR)/export.ltoir
	@tmp="$@.$$$$.tmp"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(BUILD_ENV) CARGO_TARGET_DIR=$(CUDA_BUILD_TARGET) $(CARGO) run --release --manifest-path crates/cuda/link/Cargo.toml -- $(CUDA_ARTIFACT_DIR)/export.ltoir "$$tmp" export $(CUDA_OXIDE_TARGET); \
	mv "$$tmp" $@

dev: SHELL := /bin/bash
dev: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) CARGO_TERM_COLOR=always $(CARGO) build -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins
	@started="$$(date --iso-8601=seconds)"; \
	# Do not replace this with `cargo oxide run`: it forces release opt-level=3 \
	# and touches the host crate, causing a full optimized rebuild every run. \
	$(BUILD_ENV) RUST_LOG=$(RUST_LOG) target/debug/$(BIN_NAME) 2>&1 \
		| tee >(sed -E 's/\x1B\[[0-9;]*[[:alpha:]]//g' > "$(DEV_LOG)"); \
	status=$${PIPESTATUS[0]}; \
	if [[ $$status -ne 0 ]]; then \
		$(MAKE) crash-report CRASH_SINCE="$$started" CRASH_PROFILE=debug || true; \
		echo "Debug trace: $(DEV_LOG)"; \
	fi; \
	exit $$status

qt-build: native-deps qt-native-deps cuda-artifacts
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) CARGO_TERM_COLOR=always $(CARGO) build -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

dev-qt: SHELL := /bin/bash
qt-desktop-file:
	sed -e 's|^Exec=.*|Exec=$(CURDIR)/target/debug/$(QT_BIN_NAME) %f|' -e 's|^TryExec=.*|TryExec=$(CURDIR)/target/debug/$(QT_BIN_NAME)|' $(QT_DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(APPLICATIONSDIR)/dev.shrimply.Shrimply.Qt.desktop"
	@command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true

dev-qt: qt-build qt-desktop-file
	@started="$$(date --iso-8601=seconds)"; \
	$(BUILD_ENV) RUST_LOG=$(RUST_LOG) target/debug/$(QT_BIN_NAME) 2>&1 \
		| tee >(sed -E 's/\x1B\[[0-9;]*[[:alpha:]]//g' > "$(QT_DEV_LOG)"); \
	status=$${PIPESTATUS[0]}; \
	if [[ $$status -ne 0 ]]; then \
		$(MAKE) crash-report CRASH_SINCE="$$started" CRASH_PROFILE=debug || true; \
		echo "Debug trace: $(QT_DEV_LOG)"; \
	fi; \
	exit $$status

dev-server:
	uv run --project server --locked server/src/main.py

docs:
	uv run --project docs --locked sphinx-build docs/source docs/build

docs-check:
	uv run --project docs --locked sphinx-build -W --keep-going docs/source docs/build

run: dev

run-qt: qt-build qt-desktop-file
	$(BUILD_ENV) RUST_LOG=$(RUST_LOG) target/debug/$(QT_BIN_NAME)

build: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) build -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

release: native-deps cuda-artifacts
	$(BUILD_ENV) $(CARGO) build --release -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

check: native-deps cuda-artifacts fmt source-size-check cargo-check lint server-python-check manim-python-check docs-check

components-check: native-deps qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) check -p $(GTK_COMPONENTS_PACKAGE) -p $(QT_COMPONENTS_PACKAGE) -p $(GTK_COMPONENTS_DEMO_PACKAGE) -p $(QT_COMPONENTS_DEMO_PACKAGE) --all-targets
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) clippy -p $(GTK_COMPONENTS_PACKAGE) -p $(QT_COMPONENTS_PACKAGE) -p $(GTK_COMPONENTS_DEMO_PACKAGE) -p $(QT_COMPONENTS_DEMO_PACKAGE) --all-targets -- -D warnings

gtk-components-showcase: native-deps
	$(DEV_BUILD_ENV) $(CARGO) run -p $(GTK_COMPONENTS_DEMO_PACKAGE)

qt-components-showcase: qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) run -p $(QT_COMPONENTS_DEMO_PACKAGE)

source-size-check:
	@oversized="$$(rg --files -g '!external/**' -g '!target/**' | while IFS= read -r source_file; do \
		case "$$source_file" in \
			*.rs|*.py|*.c|*.cc|*.cpp|*.cxx|*.h|*.hh|*.hpp|*.cu|*.cuh|*.wgsl|*.glsl|*.vert|*.frag|*.comp|*.slang|*.ts|*.tsx|*.js|*.jsx) \
				line_count=$$(wc -l < "$$source_file"); \
				if [ "$$line_count" -gt "$(SOURCE_LINE_LIMIT)" ]; then printf '%s: %s lines\n' "$$source_file" "$$line_count"; fi ;; \
		esac; \
	done)"; \
	if [ -n "$$oversized" ]; then printf 'Source files exceed $(SOURCE_LINE_LIMIT) lines:\n%s\n' "$$oversized"; exit 1; fi

server-python-check:
	cd server && uv run --locked pyrefly check

manim:
	cd crates/manim/manim-parser/python && uv run --python 3.14 python -m shrimply_manim $(ARGS)

manim-python-check:
	uv run --python 3.14 --project crates/manim/manim-parser/python pyrefly check --python-version 3.14 --search-path external/manim crates/manim/manim-parser/python/shrimply_manim

manim-visual-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-wgpu --test visual_parity -- --ignored --nocapture

manim-parameter-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-parser --test two_pass_parameters -- --ignored --nocapture

cargo-check:
	$(DEV_BUILD_ENV) $(CARGO) check -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

frame-rate-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-math-core frame_rate_is_the_reciprocal_of_the_latest_render_cost

video-lifecycle-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video continuous_playback_coalesces_until_an_explicit_discontinuity

transparent-fill-frame-range-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::partial_first_project_frame_uses_the_item_start_mask -- --exact --test-threads=1

transparent-fill-cache-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::cache_round_trips_evicted_project_frame_masks -- --exact --test-threads=1

transparent-fill-decoder-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-decoder tests::accurate_out_of_order_requests_map_30fps_positions_to_24fps_frames -- --exact --test-threads=1 --nocapture

transparent-fill-kernel-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::cached_mask_applies_with_the_cuda_kernel -- --exact --test-threads=1

transparent-fill-compositor-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::preview_compositor_applies_each_out_of_order_project_frame_mask -- --exact --ignored --test-threads=1

transparent-fill-playback-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::preview_uses_the_mask_for_each_project_frame -- --exact --ignored --test-threads=1 --nocapture

transparent-fill-e2e-fixture: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::generates_transparent_fill_end_to_end_fixture -- --exact --test-threads=1 --nocapture

transparent-fill-e2e-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video modifiers::transparent_fill::tests::transparent_fill_analyzes_and_renders_a_real_project_end_to_end -- --exact --ignored --test-threads=1 --nocapture

fmt:
	$(BUILD_ENV) $(CARGO) fmt
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/preview/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/anime4k/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/modifiers/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/modifiers-blur/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/modifiers-geometry/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/modifiers-matte/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/stabilization/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --manifest-path crates/cuda/export/Cargo.toml

fmt-check:
	$(BUILD_ENV) $(CARGO) fmt --check
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/preview/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/anime4k/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/modifiers/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/modifiers-blur/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/modifiers-geometry/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/modifiers-matte/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/stabilization/Cargo.toml
	$(BUILD_ENV) $(CARGO) fmt --check --manifest-path crates/cuda/export/Cargo.toml

lint:
	$(DEV_BUILD_ENV) $(CARGO) clippy -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins -- -D warnings

test: cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test

decode-ahead-benchmark:
	@test -n "$(VIDEO)" || { echo "usage: make decode-ahead-benchmark VIDEO=/path/to/video.mp4 [FRAMES=300] [LAYERS=2]" >&2; exit 1; }
	$(DEV_BUILD_ENV) $(CARGO) run -p shrimply-video --example decode_ahead_benchmark -- "$(VIDEO)" "$(or $(FRAMES),300)" "$(or $(LAYERS),2)"

paint-interpolation-test:
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-paint-interpolation

crash-report:
	@pid=""; \
	for _ in $$(seq 1 50); do \
		pid="$$(LC_ALL=C coredumpctl -q --since "$(CRASH_SINCE)" --no-legend list $(EDITOR_BIN_NAME) 2>/dev/null | awk 'END { print $$5 }')"; \
		test -z "$$pid" || break; \
		sleep 0.1; \
	done; \
	test -n "$$pid"; \
	coredumpctl -q -o $(CRASH_CORE) dump "$$pid" >/dev/null 2>&1
	@eu-stack -s -i --core=$(CRASH_CORE) --executable=target/$(CRASH_PROFILE)/$(EDITOR_BIN_NAME) > $(CRASH_STACK) 2>&1 || test $$? -eq 1
	@sed -n '1,80p' $(CRASH_STACK)
	@echo "Full crash stack: $(CRASH_STACK)"
	@echo "Core dump: $(CRASH_CORE)"

oxide-doctor:
	$(OXIDE_ENV) $(CARGO) oxide doctor

oxide-setup:
	$(OXIDE_ENV) $(CARGO) oxide setup

oxide-setup-arch:
	cd external/cuda-oxide/crates/rustc-codegen-cuda && \
		SYSROOT="$$($(RUSTUP) run $(RUST_TOOLCHAIN) rustc --print sysroot)" && \
		LIBRARY_PATH="$$SYSROOT/lib" LD_LIBRARY_PATH="$$SYSROOT/lib" \
		$(CARGO) build --lib --target host-tuple --target-dir target
	@echo
	@echo "Backend built. Export this before running make dev/build/release/check:"
	@echo "  export CUDA_OXIDE_BACKEND=$(CURDIR)/external/cuda-oxide/crates/rustc-codegen-cuda/target/x86_64-unknown-linux-gnu/debug/librustc_codegen_cuda.so"

clean-dev:
	$(CARGO) clean --profile dev

clean:
	$(CARGO) clean
	rm -rf .oxide-artifacts
	rm -rf docs/build

deps-fedora:
	$(DNF) install $(FEDORA_PACKAGES)

deps-arch:
	$(PACMAN) $(ARCH_PACKAGES)
	$(RUSTUP) toolchain install $(RUST_TOOLCHAIN) --component rust-src,rustc-dev,llvm-tools,rustfmt,clippy
	git submodule update --init --recursive --progress
	$(RUSTUP) run $(RUST_TOOLCHAIN) cargo install --path external/cuda-oxide/crates/cargo-oxide --locked

install: release
	$(INSTALL) -Dm755 target/release/$(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	$(INSTALL) -Dm755 target/release/$(EDITOR_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
	$(INSTALL) -Dm755 target/release/$(MCP_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(MCP_BIN_NAME)"
	$(INSTALL) -Dm755 $(RHUBARB_LIBRARY) "$(DESTDIR)$(BINDIR)/libshrimply-rhubarb.so"
	$(INSTALL) -d "$(DESTDIR)$(RHUBARB_RESOURCE_DIR)"
	cp -a $(RHUBARB_MODEL_SOURCE)/. "$(DESTDIR)$(RHUBARB_RESOURCE_DIR)/"
	$(INSTALL) -d "$(DESTDIR)$(RHUBARB_RESOURCE_DIR)/acoustic-model"
	cp -a $(RHUBARB_ACOUSTIC_MODEL_SOURCE)/. "$(DESTDIR)$(RHUBARB_RESOURCE_DIR)/acoustic-model/"
	sed -e 's|^Exec=.*|Exec=$(BINDIR)/$(BIN_NAME) %f|' -e 's|^TryExec=.*|TryExec=$(BINDIR)/$(BIN_NAME)|' $(DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	$(INSTALL) -Dm644 $(APP_ICON) "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
	fi
	@echo "Installed $(APP_NAME) under $(DESTDIR)$(PREFIX)"

install-codex-mcp-dev:
	@test -x "$(CURDIR)/target/debug/$(MCP_BIN_NAME)" || { echo "Run make dev first to build target/debug/$(MCP_BIN_NAME)" >&2; exit 2; }
	@test -n "$(XDG_RUNTIME_DIR)" || { echo "XDG_RUNTIME_DIR is not set" >&2; exit 2; }
	@if $(CODEX) mcp get "$(MCP_SERVER_NAME)" >/dev/null 2>&1; then $(CODEX) mcp remove "$(MCP_SERVER_NAME)"; fi
	$(CODEX) mcp add "$(MCP_SERVER_NAME)" --env XDG_RUNTIME_DIR="$(XDG_RUNTIME_DIR)" -- "$(CURDIR)/target/debug/$(MCP_BIN_NAME)"

install-agy-mcp-dev:
	@test -x "$(CURDIR)/target/debug/$(MCP_BIN_NAME)" || { echo "Run make dev first to build target/debug/$(MCP_BIN_NAME)" >&2; exit 2; }
	@test -n "$(XDG_RUNTIME_DIR)" || { echo "XDG_RUNTIME_DIR is not set" >&2; exit 2; }
	$(AGY) mcp add --env XDG_RUNTIME_DIR="$(XDG_RUNTIME_DIR)" "$(MCP_SERVER_NAME)" "$(CURDIR)/target/debug/$(MCP_BIN_NAME)"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(MCP_BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/libshrimply-rhubarb.so"
	rm -rf "$(DESTDIR)$(DATADIR)/shrimply/rhubarb"
	rm -f "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	rm -f "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.Qt.desktop"
	rm -f "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
	fi
