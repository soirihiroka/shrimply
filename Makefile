RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
HOST_OS := $(shell uname -s)
CARGO ?= $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
RUSTC ?= $(RUSTUP) run $(RUST_TOOLCHAIN) rustc
CARGO_TARGET_DIR ?= target
CUDA_HOME ?= /usr/local/cuda
CUDA_TOOLKIT_PATH ?= $(CUDA_HOME)
CUDA_TARGET ?= sm_86
CUDA_IMAGE_FORMAT ?= cubin
CUDA_PTX_TARGET ?= compute_50
CUDA_HOST_CXX ?= g++-15
CUDA_ALLOW_UNSUPPORTED_COMPILER ?=
SOURCE_LINE_LIMIT ?= 2000
OPTIX_ROOT ?= $(CURDIR)/external/optix-dev
DNF ?= sudo dnf
INSTALL ?= install
PKG_CONFIG ?= /usr/bin/pkg-config
PKG_CONFIG_PATH ?= /usr/lib64/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig
QT_QMAKE ?= qmake6
BUILD_ENV := CUDA_HOME=$(CUDA_HOME) CUDA_TOOLKIT_PATH=$(CUDA_TOOLKIT_PATH) CUDA_IMAGE_FORMAT=$(CUDA_IMAGE_FORMAT) CUDA_TARGET=$(CUDA_TARGET) CUDA_PTX_TARGET=$(CUDA_PTX_TARGET) CUDA_HOST_CXX=$(CUDA_HOST_CXX) CUDA_ALLOW_UNSUPPORTED_COMPILER=$(CUDA_ALLOW_UNSUPPORTED_COMPILER) PATH=$(CUDA_HOME)/bin:$(PATH) PKG_CONFIG=$(PKG_CONFIG) PKG_CONFIG_PATH=$(PKG_CONFIG_PATH) OPTIX_ROOT=$(OPTIX_ROOT)
BUILD_ENV += LD_LIBRARY_PATH="/run/host/usr/local/libclang-deps:$${LD_LIBRARY_PATH}"
BUILD_ENV += BINDGEN_EXTRA_CLANG_ARGS="$(if $(wildcard /run/host/usr/lib/llvm-18/lib/clang/18/include),-isystem /run/host/usr/lib/llvm-18/lib/clang/18/include)"
RUST_LIBDIR := $(shell $(RUSTC) --print target-libdir)
DEV_RUSTFLAGS ?= -C prefer-dynamic -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)
DEV_BUILD_ENV := $(BUILD_ENV) RUSTFLAGS="$(DEV_RUSTFLAGS)"

APP_NAME := Shrimply
BIN_NAME := shrimply
EDITOR_BIN_NAME := shrimply-editor
EDITOR_PACKAGE := shrimply-editor-gtk
QT_EDITOR_PACKAGE := shrimply-editor-qt
LAUNCHER_PACKAGE := shrimply-launcher-gtk
QT_LAUNCHER_PACKAGE := shrimply-launcher-qt
APPKIT_LAUNCHER_PACKAGE := shrimply-launcher-appkit
APPKIT_EDITOR_PACKAGE := shrimply-editor-appkit
APPKIT_COMPONENT_METAL_PACKAGE := shrimply-framegraph-appkit
FRAMEGRAPH_CORE_PACKAGE := shrimply-framegraph-skia
APPKIT_COMPONENTS_PACKAGE := shrimply-components-appkit
APPKIT_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-appkit
GTK_COMPONENTS_PACKAGE := shrimply-components-gtk
QT_COMPONENTS_PACKAGE := shrimply-components-qt
GTK_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-gtk
QT_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-qt
QT_BIN_NAME := shrimply-qt
APPKIT_BIN_NAME := shrimply-appkit
APPKIT_EDITOR_BIN_NAME := shrimply-editor-appkit
QT_EDITOR_BIN_NAME := shrimply-editor-qt
MCP_PACKAGE := shrimply-mcp
MCP_BIN_NAME := shrimply-mcp
MCP_SERVER_NAME ?= shrimply
CODEX ?= codex
AGY ?= agy
RUST_LOG ?= info,shrimply=debug,shrimply_editor=debug,shrimply_launcher=debug,shrimply_launcher_qt=debug,shrimply_timeline_gtk=debug
DEV_LOG ?= target/$(BIN_NAME)-dev.log
QT_DEV_LOG ?= target/$(QT_BIN_NAME)-dev.log
CRASH_CORE ?= target/$(EDITOR_BIN_NAME).core
CRASH_STACK ?= target/$(EDITOR_BIN_NAME).stack
CRASH_PROFILE ?= debug
CRASH_SINCE ?= -1 day
DOCKER ?= docker
FLATPAK_GTK_IMAGE ?= shrimply-flatpak-gtk-builder
FLATPAK_GTK_CACHE ?= $(CURDIR)/target/flatpak-gtk
FLATPAK_BUNDLE := dist/shrimply-gtk.flatpak

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
APPLICATIONSDIR ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/apps
DESKTOP_FILE := assets/dev.shrimply.Shrimply.desktop
QT_DESKTOP_FILE := assets/dev.shrimply.Shrimply.Qt.desktop
APP_ICON := assets/icons/dev.shrimply.Shrimply.svg
APPKIT_ICON_SOURCE := assets/icons/dev.shrimply.Shrimply-macos.svg
APPKIT_ICON := assets/icons/dev.shrimply.Shrimply.png
APPKIT_ICON_SIZE := 512
APPKIT_DEPLOYMENT_TARGET ?= 15.0
RSVG_CONVERT ?= rsvg-convert
LIP_SYNC_MODEL := $(CARGO_TARGET_DIR)/release/res/lip-sync/pocketsphinx-ci.model
LIP_SYNC_RESOURCE_DIR := $(DATADIR)/shrimply/lip-sync
LIP_SYNC_LICENSE_DIR := $(DATADIR)/licenses/shrimply
ICONS_RESOURCE_DIR := $(DATADIR)/shrimply/icons

FEDORA_PACKAGES := \
	rust \
	cargo \
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
	freetype-devel \
	qt6-qtbase-devel \
	qt6-qtdeclarative-devel

.PHONY: native-deps qt-native-deps qt-desktop-file desktop-icon cuda-target-check cuda-artifacts dev dev-mac qt-build dev-qt dev-server docs docs-check run run-qt build release check components-check gtk-components-showcase qt-components-showcase server-python-check manim manim-python-check manim-parameter-check cargo-check fmt fmt-check lint test frame-rate-test video-lifecycle-test transparent-fill-frame-range-test transparent-fill-decoder-test transparent-fill-kernel-test transparent-fill-compositor-test transparent-fill-playback-test transparent-fill-e2e-fixture transparent-fill-e2e-test decode-ahead-benchmark paint-interpolation-test crash-report clean-dev clean deps-fedora deps-fedora-qt qt-release install install-qt install-codex-mcp-dev install-agy-mcp-dev uninstall uninstall-qt flatpak-gtk
native-deps:
	@$(PKG_CONFIG) --exists rubberband || { echo "Missing Rubber Band development files (pkg-config: rubberband)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists libpipewire-0.3 || { echo "Missing PipeWire development files (pkg-config: libpipewire-0.3)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists poppler-glib || { echo "Missing Poppler GLib development files (pkg-config: poppler-glib)" >&2; exit 1; }

qt-native-deps:
	@command -v $(QT_QMAKE) >/dev/null 2>&1 || { echo "Missing Qt 6 qmake ($(QT_QMAKE))" >&2; exit 1; }
	@version="$$($(QT_QMAKE) -query QT_VERSION)"; case "$$version" in 6.*) echo "Using Qt $$version via $(QT_QMAKE)" ;; *) echo "$(QT_QMAKE) selected unsupported Qt $$version; Qt 6 is required" >&2; exit 1 ;; esac
	@$(PKG_CONFIG) --exists Qt6Core Qt6Gui Qt6Qml Qt6Quick Qt6QuickControls2 Qt6OpenGL || { echo "Missing Qt 6 Quick/OpenGL development files" >&2; exit 1; }

cuda-target-check:
	@test "$$(uname -s)" = Linux || { echo "CUDA kernels require Linux" >&2; exit 1; }
	@case "$(CUDA_IMAGE_FORMAT)" in \
		(cubin) case "$(CUDA_TARGET)" in sm_*) ;; (*) echo "CUDA_TARGET=$(CUDA_TARGET) must be a physical SM architecture" >&2; exit 1 ;; esac ;; \
		(ptx) case "$(CUDA_PTX_TARGET)" in compute_*) ;; (*) echo "CUDA_PTX_TARGET=$(CUDA_PTX_TARGET) must be a virtual compute architecture" >&2; exit 1 ;; esac ;; \
		(*) echo "CUDA_IMAGE_FORMAT=$(CUDA_IMAGE_FORMAT) is unsupported; expected cubin or ptx" >&2; exit 1 ;; \
	esac

cuda-artifacts: cuda-target-check
	$(BUILD_ENV) $(CARGO) build -p shrimply-render-kernels-cuda

dev: SHELL := /bin/bash
desktop-icon:
	$(INSTALL) -Dm644 $(APP_ICON) "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
	fi

dev: desktop-icon native-deps cuda-artifacts
	$(DEV_BUILD_ENV) CARGO_TERM_COLOR=always $(CARGO) build -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins
	@started="$$(date --iso-8601=seconds)"; \
	$(BUILD_ENV) RUST_LOG=$(RUST_LOG) target/debug/$(BIN_NAME) 2>&1 \
		| tee >(sed -E 's/\x1B\[[0-9;]*[[:alpha:]]//g' > "$(DEV_LOG)"); \
	status=$${PIPESTATUS[0]}; \
	if [[ $$status -ne 0 ]]; then \
		$(MAKE) crash-report CRASH_SINCE="$$started" CRASH_PROFILE=debug || true; \
		echo "Debug trace: $(DEV_LOG)"; \
	fi; \
	exit $$status

APPKIT_BUILD_ENV = RUSTFLAGS="-C prefer-dynamic -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)" LIBRARY_PATH="$$(brew --prefix)/lib" PKG_CONFIG="$$(brew --prefix pkgconf)/bin/pkg-config" CLANG_PATH="$$(brew --prefix llvm@18)/bin/clang" LIBCLANG_PATH="$$(brew --prefix llvm@18)/lib"

.PHONY: appkit-build appkit-release appkit-check appkit-lint appkit-components-check appkit-components-showcase
$(APPKIT_ICON): $(APPKIT_ICON_SOURCE)
	$(RSVG_CONVERT) --width $(APPKIT_ICON_SIZE) --height $(APPKIT_ICON_SIZE) $< --output $@

appkit-build: $(APPKIT_ICON)
	@test "$$(uname -s)" = Darwin || { echo "dev-mac requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) build -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins

appkit-release: $(APPKIT_ICON)
	@test "$$(uname -s)" = Darwin || { echo "AppKit release requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) RUSTFLAGS="" CARGO_TERM_COLOR=always MACOSX_DEPLOYMENT_TARGET=$(APPKIT_DEPLOYMENT_TARGET) $(CARGO) build --release -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins

appkit-check: appkit-build
	$(APPKIT_BUILD_ENV) $(CARGO) check -p $(APPKIT_EDITOR_PACKAGE) -p $(APPKIT_LAUNCHER_PACKAGE) -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(APPKIT_COMPONENT_METAL_PACKAGE) -p $(APPKIT_COMPONENTS_PACKAGE) -p $(APPKIT_COMPONENTS_DEMO_PACKAGE) --all-targets
	$(MAKE) appkit-lint

appkit-lint:
	@test "$$(uname -s)" = Darwin || { echo "AppKit lint requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) clippy -p $(APPKIT_EDITOR_PACKAGE) -p $(APPKIT_LAUNCHER_PACKAGE) -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(APPKIT_COMPONENT_METAL_PACKAGE) -p $(APPKIT_COMPONENTS_PACKAGE) -p $(APPKIT_COMPONENTS_DEMO_PACKAGE) --all-targets -- -D warnings

appkit-components-check:
	@test "$$(uname -s)" = Darwin || { echo "AppKit components require macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) check -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(APPKIT_COMPONENT_METAL_PACKAGE) -p $(APPKIT_COMPONENTS_PACKAGE) -p $(APPKIT_COMPONENTS_DEMO_PACKAGE) --all-targets
	$(APPKIT_BUILD_ENV) $(CARGO) clippy -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(APPKIT_COMPONENT_METAL_PACKAGE) -p $(APPKIT_COMPONENTS_PACKAGE) -p $(APPKIT_COMPONENTS_DEMO_PACKAGE) --all-targets -- -D warnings

appkit-components-showcase:
	@test "$$(uname -s)" = Darwin || { echo "AppKit components require macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) run -p $(APPKIT_COMPONENTS_DEMO_PACKAGE)

dev-mac: appkit-build
	RUST_LOG=$(RUST_LOG) target/debug/$(APPKIT_BIN_NAME)

qt-build: native-deps qt-native-deps cuda-artifacts
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) CARGO_TERM_COLOR=always $(CARGO) build -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

dev-qt: SHELL := /bin/bash
qt-desktop-file: desktop-icon
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

ifeq ($(HOST_OS),Linux)
check: native-deps qt-native-deps cuda-artifacts fmt source-size-check cargo-check lint server-python-check manim-python-check docs-check
else ifeq ($(HOST_OS),Darwin)
check: appkit-check fmt source-size-check
else
check:
	@echo "make check supports Linux and macOS; unsupported platform: $(HOST_OS)" >&2
	@exit 1
endif

components-check: native-deps qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) check -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(GTK_COMPONENTS_PACKAGE) -p $(QT_COMPONENTS_PACKAGE) -p $(GTK_COMPONENTS_DEMO_PACKAGE) -p $(QT_COMPONENTS_DEMO_PACKAGE) --all-targets
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) clippy -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(GTK_COMPONENTS_PACKAGE) -p $(QT_COMPONENTS_PACKAGE) -p $(GTK_COMPONENTS_DEMO_PACKAGE) -p $(QT_COMPONENTS_DEMO_PACKAGE) --all-targets -- -D warnings

gtk-components-showcase: native-deps
	$(DEV_BUILD_ENV) $(CARGO) run -p $(GTK_COMPONENTS_DEMO_PACKAGE)

qt-components-showcase: qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) run -p $(QT_COMPONENTS_DEMO_PACKAGE)

source-size-check:
	@oversized="$$(rg --files -g '!external/**' -g '!target/**' | while IFS= read -r source_file; do \
		case "$$source_file" in \
			(*.rs|*.py|*.c|*.cc|*.cpp|*.cxx|*.h|*.hh|*.hpp|*.cu|*.cuh|*.wgsl|*.glsl|*.vert|*.frag|*.comp|*.slang|*.ts|*.tsx|*.js|*.jsx) \
				line_count=$$(wc -l < "$$source_file"); \
				if [ "$$line_count" -gt "$(SOURCE_LINE_LIMIT)" ]; then printf '%s: %s lines\n' "$$source_file" "$$line_count"; fi ;; \
		esac; \
	done)"; \
	if [ -n "$$oversized" ]; then printf 'Source files exceed $(SOURCE_LINE_LIMIT) lines:\n%s\n' "$$oversized"; exit 1; fi

server-python-check:
	cd server && uv run --locked pyrefly check

manim:
	cd crates/media/visual/manim/manim-bridge/python && uv run --python 3.14 python -m shrimply_manim $(ARGS)

manim-python-check:
	uv run --python 3.14 --project crates/media/visual/manim/manim-bridge/python pyrefly check --python-version 3.14 --search-path external/manim crates/media/visual/manim/manim-bridge/python/shrimply_manim

manim-visual-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-wgpu --test visual_parity -- --ignored --nocapture

manim-parameter-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-bridge --test two_pass_parameters -- --ignored --nocapture

cargo-check: native-deps qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) check -p $(EDITOR_PACKAGE) -p $(QT_EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

frame-rate-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-math-core frame_rate_is_the_reciprocal_of_the_latest_render_cost

video-lifecycle-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda continuous_playback_coalesces_until_an_explicit_discontinuity

transparent-fill-frame-range-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::partial_first_project_frame_uses_the_item_start_mask -- --exact --test-threads=1

transparent-fill-cache-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::cache_round_trips_evicted_project_frame_masks -- --exact --test-threads=1

transparent-fill-decoder-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-decoder tests::accurate_out_of_order_requests_map_30fps_positions_to_24fps_frames -- --exact --test-threads=1 --nocapture

transparent-fill-kernel-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::cached_mask_applies_with_the_cuda_kernel -- --exact --test-threads=1

transparent-fill-compositor-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::preview_compositor_applies_each_out_of_order_project_frame_mask -- --exact --ignored --test-threads=1

transparent-fill-playback-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::preview_uses_the_mask_for_each_project_frame -- --exact --ignored --test-threads=1 --nocapture

transparent-fill-e2e-fixture: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::generates_transparent_fill_end_to_end_fixture -- --exact --test-threads=1 --nocapture

transparent-fill-e2e-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-visual-cuda modifiers::transparent_fill::tests::transparent_fill_analyzes_and_renders_a_real_project_end_to_end -- --exact --ignored --test-threads=1 --nocapture

fmt:
	$(BUILD_ENV) $(CARGO) fmt

fmt-check:
	$(BUILD_ENV) $(CARGO) fmt --check

lint: native-deps qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) clippy -p $(EDITOR_PACKAGE) -p $(QT_EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins -- -D warnings

test: cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test

decode-ahead-benchmark:
	@test -n "$(VIDEO)" || { echo "usage: make decode-ahead-benchmark VIDEO=/path/to/video.mp4 [FRAMES=300] [LAYERS=2]" >&2; exit 1; }
	$(DEV_BUILD_ENV) $(CARGO) run -p shrimply-visual-cuda --example decode_ahead_benchmark -- "$(VIDEO)" "$(or $(FRAMES),300)" "$(or $(LAYERS),2)"

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

clean-dev:
	$(CARGO) clean --profile dev

clean:
	$(CARGO) clean
	rm -rf .slang-artifacts
	rm -rf docs/build

deps-fedora:
	$(DNF) install $(FEDORA_PACKAGES)

qt-release: native-deps qt-native-deps cuda-artifacts
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) CARGO_TERM_COLOR=always $(CARGO) build --release -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE)

install: release desktop-icon
	$(INSTALL) -Dm755 $(CARGO_TARGET_DIR)/release/$(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	$(INSTALL) -Dm755 $(CARGO_TARGET_DIR)/release/$(EDITOR_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
	$(INSTALL) -Dm755 $(CARGO_TARGET_DIR)/release/$(MCP_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(MCP_BIN_NAME)"
	$(INSTALL) -Dm644 $(LIP_SYNC_MODEL) "$(DESTDIR)$(LIP_SYNC_RESOURCE_DIR)/pocketsphinx-ci.model"
	$(INSTALL) -Dm644 vendor/pocketsphinx/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	$(INSTALL) -Dm644 vendor/pocketsphinx/MODEL-LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	$(INSTALL) -Dm644 vendor/rhubarb-lip-sync/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	$(INSTALL) -d "$(DESTDIR)$(ICONS_RESOURCE_DIR)"
	cp -a assets/icons/. "$(DESTDIR)$(ICONS_RESOURCE_DIR)/"
	sed -e 's|^Exec=.*|Exec=$(BINDIR)/$(BIN_NAME) %f|' -e 's|^TryExec=.*|TryExec=$(BINDIR)/$(BIN_NAME)|' $(DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
	fi
	@echo "Installed $(APP_NAME) under $(DESTDIR)$(PREFIX)"

flatpak-gtk:
	@command -v $(DOCKER) >/dev/null 2>&1 || { echo "Missing Docker ($(DOCKER))" >&2; exit 1; }
	mkdir -p "$(dir $(FLATPAK_BUNDLE))" "$(FLATPAK_GTK_CACHE)"
	$(DOCKER) build -f Dockerfile.flatpak-gtk -t "$(FLATPAK_GTK_IMAGE)" .
	$(DOCKER) run --rm --privileged \
		--env OUTPUT_UID="$$(id -u)" \
		--env OUTPUT_GID="$$(id -g)" \
		--volume "$(FLATPAK_GTK_CACHE):/flatpak:Z" \
		--volume "$(CURDIR)/$(dir $(FLATPAK_BUNDLE)):/output:Z" \
		"$(FLATPAK_GTK_IMAGE)"
	@echo "Flatpak bundle: $(FLATPAK_BUNDLE)"

install-qt: qt-release desktop-icon
	$(INSTALL) -Dm755 target/release/$(QT_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(QT_BIN_NAME)"
	$(INSTALL) -Dm755 target/release/$(QT_EDITOR_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(QT_EDITOR_BIN_NAME)"
	$(INSTALL) -Dm644 $(LIP_SYNC_MODEL) "$(DESTDIR)$(LIP_SYNC_RESOURCE_DIR)/pocketsphinx-ci.model"
	$(INSTALL) -Dm644 vendor/pocketsphinx/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	$(INSTALL) -Dm644 vendor/pocketsphinx/MODEL-LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	$(INSTALL) -Dm644 vendor/rhubarb-lip-sync/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	sed -e 's|^Exec=.*|Exec=$(BINDIR)/$(QT_BIN_NAME) %f|' -e 's|^TryExec=.*|TryExec=$(BINDIR)/$(QT_BIN_NAME)|' $(QT_DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.Qt.desktop"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
	fi
	@echo "Installed Qt launcher/editor under $(DESTDIR)$(PREFIX)"

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
	rm -rf "$(DESTDIR)$(LIP_SYNC_RESOURCE_DIR)"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	rm -rf "$(DESTDIR)$(ICONS_RESOURCE_DIR)"
	rm -f "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	rm -f "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
	fi

uninstall-qt:
	rm -f "$(DESTDIR)$(BINDIR)/$(QT_BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(QT_EDITOR_BIN_NAME)"
	rm -f "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.Qt.desktop"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
	fi
