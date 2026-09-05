RUSTUP ?= rustup
RUST_TOOLCHAIN ?= nightly-2026-04-03
CARGO := $(RUSTUP) run $(RUST_TOOLCHAIN) cargo
CUDA_HOME ?= /usr/local/cuda
CUDA_TOOLKIT_PATH ?= $(CUDA_HOME)
CUDA_TARGET ?= sm_86
CUDA_HOST_CXX ?= g++-15
SOURCE_LINE_LIMIT ?= 2000
SLANG_SOURCE_DIR ?= $(CURDIR)/external/slang
SLANG_BUILD_DIR ?= $(SLANG_SOURCE_DIR)/build
OPTIX_ROOT ?= $(CURDIR)/external/optix-dev
DNF ?= sudo dnf
INSTALL ?= install
PKG_CONFIG ?= /usr/bin/pkg-config
PKG_CONFIG_PATH ?= /usr/lib64/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig
QT_QMAKE ?= qmake6
SLANG_LIBRARY_ENV = LD_LIBRARY_PATH="$(SLANG_BUILD_DIR)/Release/lib:$${LD_LIBRARY_PATH}" DYLD_LIBRARY_PATH="$(SLANG_BUILD_DIR)/Release/lib:$${DYLD_LIBRARY_PATH}"
BUILD_ENV := CUDA_HOME=$(CUDA_HOME) CUDA_TOOLKIT_PATH=$(CUDA_TOOLKIT_PATH) PATH=$(CUDA_HOME)/bin:$(PATH) PKG_CONFIG=$(PKG_CONFIG) PKG_CONFIG_PATH=$(PKG_CONFIG_PATH) SLANG_SOURCE_DIR=$(SLANG_SOURCE_DIR) SLANG_BUILD_DIR=$(SLANG_BUILD_DIR) OPTIX_ROOT=$(OPTIX_ROOT)
BUILD_ENV += $(SLANG_LIBRARY_ENV)
RUST_LIBDIR := $(shell $(RUSTUP) run $(RUST_TOOLCHAIN) rustc --print target-libdir)
DEV_RUSTFLAGS ?= -C prefer-dynamic -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)
DEV_BUILD_ENV := $(BUILD_ENV) RUSTFLAGS="$(DEV_RUSTFLAGS)"
SLANG_COMPILER_STAMP := $(SLANG_BUILD_DIR)/.shrimply-compiler
SLANG_CONFIGURE_STAMP := $(SLANG_BUILD_DIR)/.shrimply-configure
SLANG_GIT_HEAD := $(shell git -C $(SLANG_SOURCE_DIR) rev-parse --git-path HEAD 2>/dev/null)
SLANG_GIT_REF := $(shell ref=$$(git -C $(SLANG_SOURCE_DIR) symbolic-ref -q HEAD 2>/dev/null); test -z "$$ref" || git -C $(SLANG_SOURCE_DIR) rev-parse --git-path "$$ref")

APP_NAME := Shrimply
BIN_NAME := shrimply
EDITOR_BIN_NAME := shrimply-editor
EDITOR_PACKAGE := shrimply-editor-gtk
QT_EDITOR_PACKAGE := shrimply-editor-qt
LAUNCHER_PACKAGE := shrimply-launcher-gtk
QT_LAUNCHER_PACKAGE := shrimply-launcher-qt
APPKIT_LAUNCHER_PACKAGE := shrimply-launcher-appkit
APPKIT_EDITOR_PACKAGE := shrimply-editor-appkit
GTK_COMPONENTS_PACKAGE := shrimply-gtk-components
QT_COMPONENTS_PACKAGE := shrimply-qt-components
GTK_COMPONENTS_DEMO_PACKAGE := shrimply-gtk-components-demo
QT_COMPONENTS_DEMO_PACKAGE := shrimply-qt-components-demo
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

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
APPLICATIONSDIR ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/apps
DESKTOP_FILE := assets/dev.shrimply.Shrimply.desktop
QT_DESKTOP_FILE := assets/dev.shrimply.Shrimply.Qt.desktop
APP_ICON := assets/icons/dev.shrimply.Shrimply.svg
APPKIT_ICON := assets/icons/dev.shrimply.Shrimply.png
APPKIT_ICON_SIZE := 512
RSVG_CONVERT ?= rsvg-convert
LIP_SYNC_MODEL := target/release/res/lip-sync/pocketsphinx-ci.model
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

.PHONY: native-deps qt-native-deps qt-desktop-file cuda-target-check cuda-artifacts dev dev-mac qt-build dev-qt dev-server docs docs-check run run-qt build release check components-check gtk-components-showcase qt-components-showcase server-python-check manim manim-python-check manim-parameter-check cargo-check fmt fmt-check lint test frame-rate-test video-lifecycle-test transparent-fill-frame-range-test transparent-fill-decoder-test transparent-fill-kernel-test transparent-fill-compositor-test transparent-fill-playback-test transparent-fill-e2e-fixture transparent-fill-e2e-test decode-ahead-benchmark paint-interpolation-test crash-report clean-dev clean deps-fedora deps-fedora-qt qt-release install install-qt install-codex-mcp-dev install-agy-mcp-dev uninstall uninstall-qt dist-image dist
native-deps:
	@$(PKG_CONFIG) --exists rubberband || { echo "Missing Rubber Band development files (pkg-config: rubberband)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists libpipewire-0.3 || { echo "Missing PipeWire development files (pkg-config: libpipewire-0.3)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists poppler-glib || { echo "Missing Poppler GLib development files (pkg-config: poppler-glib)" >&2; exit 1; }

qt-native-deps:
	@command -v $(QT_QMAKE) >/dev/null 2>&1 || { echo "Missing Qt 6 qmake ($(QT_QMAKE))" >&2; exit 1; }
	@version="$$($(QT_QMAKE) -query QT_VERSION)"; case "$$version" in 6.*) echo "Using Qt $$version via $(QT_QMAKE)" ;; *) echo "$(QT_QMAKE) selected unsupported Qt $$version; Qt 6 is required" >&2; exit 1 ;; esac
	@$(PKG_CONFIG) --exists Qt6Core Qt6Gui Qt6Qml Qt6Quick Qt6QuickControls2 Qt6OpenGL || { echo "Missing Qt 6 Quick/OpenGL development files" >&2; exit 1; }

slang-compiler: $(SLANG_COMPILER_STAMP)

$(SLANG_CONFIGURE_STAMP): $(SLANG_SOURCE_DIR)/CMakeLists.txt Makefile
	cmake -S $(SLANG_SOURCE_DIR) -B $(SLANG_BUILD_DIR) -G "Ninja Multi-Config" -DSLANG_ENABLE_SLANGC=OFF -DSLANG_ENABLE_SLANG_RHI=OFF -DSLANG_ENABLE_GFX=OFF -DSLANG_ENABLE_TESTS=OFF -DSLANG_ENABLE_EXAMPLES=OFF -DSLANG_ENABLE_SLANGD=OFF -DSLANG_ENABLE_SLANGI=OFF -DSLANG_ENABLE_SLANGRT=OFF -DSLANG_ENABLE_SPLIT_DEBUG_INFO=OFF -DSLANG_ENABLE_SLANG_GLSLANG=ON -DSLANG_ENABLE_REPLAYER=OFF -DSLANG_SLANG_LLVM_FLAVOR=DISABLE -DSLANG_ENABLE_DXIL=OFF
	@touch $@

$(SLANG_COMPILER_STAMP): $(SLANG_CONFIGURE_STAMP) $(SLANG_GIT_HEAD) $(SLANG_GIT_REF)
	cmake --build $(SLANG_BUILD_DIR) --config Release --target slang slang-glslang
	@touch $@

cuda-target-check:
	@test "$$(uname -s)" = Linux || { echo "CUDA kernels require Linux" >&2; exit 1; }
	@test "$(CUDA_TARGET)" = sm_86 || { echo "CUDA_TARGET=$(CUDA_TARGET) is unsupported: host binaries embed sm_86 CUDA artifacts" >&2; exit 1; }

cuda-artifacts: cuda-target-check slang-compiler
	$(BUILD_ENV) CUDA_TARGET=$(CUDA_TARGET) CUDA_HOST_CXX=$(CUDA_HOST_CXX) $(CARGO) build -p shrimply-render-cuda

dev: SHELL := /bin/bash
dev: native-deps cuda-artifacts
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

APPKIT_BUILD_ENV = $(SLANG_LIBRARY_ENV) RUSTFLAGS="-C prefer-dynamic -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)" LIBRARY_PATH="$$(brew --prefix)/lib" PKG_CONFIG="$$(brew --prefix pkgconf)/bin/pkg-config" CLANG_PATH="$$(brew --prefix llvm@18)/bin/clang" LIBCLANG_PATH="$$(brew --prefix llvm@18)/lib" SLANG_SOURCE_DIR=$(SLANG_SOURCE_DIR) SLANG_BUILD_DIR=$(SLANG_BUILD_DIR)

.PHONY: appkit-build appkit-check
$(APPKIT_ICON): $(APP_ICON)
	$(RSVG_CONVERT) --width $(APPKIT_ICON_SIZE) --height $(APPKIT_ICON_SIZE) $< --output $@

appkit-build: $(APPKIT_ICON)
	@test "$$(uname -s)" = Darwin || { echo "dev-mac requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) build -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins

appkit-check: appkit-build
	$(APPKIT_BUILD_ENV) $(CARGO) check -p $(APPKIT_EDITOR_PACKAGE) -p $(APPKIT_LAUNCHER_PACKAGE) --all-targets
	$(APPKIT_BUILD_ENV) $(CARGO) clippy -p $(APPKIT_EDITOR_PACKAGE) -p $(APPKIT_LAUNCHER_PACKAGE) --all-targets -- -D warnings

dev-mac: appkit-build
	RUST_LOG=$(RUST_LOG) target/debug/$(APPKIT_BIN_NAME)

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

check: native-deps qt-native-deps cuda-artifacts fmt source-size-check cargo-check lint server-python-check manim-python-check docs-check

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

cargo-check: native-deps qt-native-deps slang-compiler
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) check -p $(EDITOR_PACKAGE) -p $(QT_EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) -p $(MCP_PACKAGE) --bins

frame-rate-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-math-core frame_rate_is_the_reciprocal_of_the_latest_render_cost

video-lifecycle-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda continuous_playback_coalesces_until_an_explicit_discontinuity

transparent-fill-frame-range-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::partial_first_project_frame_uses_the_item_start_mask -- --exact --test-threads=1

transparent-fill-cache-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::cache_round_trips_evicted_project_frame_masks -- --exact --test-threads=1

transparent-fill-decoder-test: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-decoder tests::accurate_out_of_order_requests_map_30fps_positions_to_24fps_frames -- --exact --test-threads=1 --nocapture

transparent-fill-kernel-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::cached_mask_applies_with_the_cuda_kernel -- --exact --test-threads=1

transparent-fill-compositor-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::preview_compositor_applies_each_out_of_order_project_frame_mask -- --exact --ignored --test-threads=1

transparent-fill-playback-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::preview_uses_the_mask_for_each_project_frame -- --exact --ignored --test-threads=1 --nocapture

transparent-fill-e2e-fixture: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::generates_transparent_fill_end_to_end_fixture -- --exact --test-threads=1 --nocapture

transparent-fill-e2e-test: native-deps cuda-artifacts
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-video-cuda modifiers::transparent_fill::tests::transparent_fill_analyzes_and_renders_a_real_project_end_to_end -- --exact --ignored --test-threads=1 --nocapture

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
	$(DEV_BUILD_ENV) $(CARGO) run -p shrimply-video-cuda --example decode_ahead_benchmark -- "$(VIDEO)" "$(or $(FRAMES),300)" "$(or $(LAYERS),2)"

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

install: release
	$(INSTALL) -Dm755 target/release/$(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	$(INSTALL) -Dm755 target/release/$(EDITOR_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
	$(INSTALL) -Dm755 target/release/$(MCP_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(MCP_BIN_NAME)"
	$(INSTALL) -Dm644 $(LIP_SYNC_MODEL) "$(DESTDIR)$(LIP_SYNC_RESOURCE_DIR)/pocketsphinx-ci.model"
	$(INSTALL) -Dm644 vendor/pocketsphinx/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	$(INSTALL) -Dm644 vendor/pocketsphinx/MODEL-LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	$(INSTALL) -Dm644 vendor/rhubarb-lip-sync/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	$(INSTALL) -d "$(DESTDIR)$(ICONS_RESOURCE_DIR)"
	cp -a assets/icons/. "$(DESTDIR)$(ICONS_RESOURCE_DIR)/"
	sed -e 's|^Exec=.*|Exec=$(BINDIR)/$(BIN_NAME) %f|' -e 's|^TryExec=.*|TryExec=$(BINDIR)/$(BIN_NAME)|' $(DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	$(INSTALL) -Dm644 $(APP_ICON) "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
	fi
	@echo "Installed $(APP_NAME) under $(DESTDIR)$(PREFIX)"

install-qt: qt-release
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
