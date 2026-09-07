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
DOCKER ?= docker
CUDA_ARTIFACTS_DOCKERFILE ?= packaging/docker/cuda-artifacts.Dockerfile
CUDA_ARTIFACTS_IMAGE ?= shrimply-cuda-artifacts
CUDA_ARTIFACTS_CONTAINER ?= shrimply-cuda-artifacts-run
CUDA_ARTIFACTS_PREBUILT_DIR ?= crates/render-cuda/prebuilt/$(CUDA_TARGET)
FLATPAK ?= flatpak
FLATPAK_BUILDER ?= flatpak-builder
FLATPAK_RUNTIME_VERSION ?= 50
FLATPAK_MANIFEST ?= packaging/flatpak/dev.shrimply.Shrimply.yaml
FLATPAK_BUILD_DIR ?= build
# Matches org.gnome.Sdk//50's own base (confirmed via
# `flatpak info --show-metadata org.gnome.Sdk//50`: its GL/GStreamer/etc
# extension points are all pinned to 25.08) -- the rust-nightly extension has
# no "50" branch of its own, see TODO.md M5.
FLATPAK_SDK_EXTENSION_BRANCH ?= 25.08
PKG_CONFIG ?= /usr/bin/pkg-config
PKG_CONFIG_PATH ?= /usr/lib64/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig
QT_QMAKE ?= qmake6
APPSTREAMCLI ?= appstreamcli
DESKTOP_FILE_VALIDATE ?= desktop-file-validate
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
APPKIT_COMPONENT_METAL_PACKAGE := shrimply-component-metal
FRAMEGRAPH_CORE_PACKAGE := shrimply-framegraph-core
APPKIT_COMPONENTS_PACKAGE := shrimply-components-appkit
APPKIT_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-appkit
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
APPKIT_ICON_SOURCE := assets/icons/dev.shrimply.Shrimply-macos.svg
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

.PHONY: native-deps qt-native-deps desktop-icon qt-desktop-file cuda-target-check cuda-artifacts cuda-artifacts-image flatpak-sdk flatpak-submodules flatpak-cuda-vendor flatpak-skeleton flatpak-bootstrap flatpak-rust-sdk flatpak-llvm-sdk flatpak-rust-check dev dev-mac qt-build dev-qt dev-server docs docs-check run run-qt build release check components-check gtk-components-showcase qt-components-showcase server-python-check manim manim-python-check manim-parameter-check metainfo-check desktop-file-check cargo-check fmt fmt-check lint test frame-rate-test video-lifecycle-test transparent-fill-frame-range-test transparent-fill-decoder-test transparent-fill-kernel-test transparent-fill-compositor-test transparent-fill-playback-test transparent-fill-e2e-fixture transparent-fill-e2e-test decode-ahead-benchmark paint-interpolation-test crash-report clean-dev clean deps-fedora deps-fedora-qt qt-release install install-qt install-codex-mcp-dev install-agy-mcp-dev uninstall uninstall-qt dist
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

# Occasional, heavy, explicit-only: builds the prebuilt .cubin files vendored
# for the flatpak sandbox (M4 — see TODO.md), where nvcc never runs. Not a
# dependency of `check`/`dev`/`cuda-artifacts` itself. Runs `make
# cuda-artifacts` inside a Docker image that has nvcc (this machine has no
# GPU driver in the container, same as the top-level Dockerfile), then copies
# the resulting cubins out to crates/render-cuda/prebuilt/$(CUDA_TARGET)/,
# which build.rs picks up automatically on the next `cargo build -p
# shrimply-render-cuda` (in-sandbox or not).
cuda-artifacts-image:
	@command -v $(DOCKER) >/dev/null 2>&1 || { echo "Installing docker..."; $(PACMAN) -S --noconfirm docker; sudo systemctl enable --now docker; }
	$(DOCKER) build -f $(CUDA_ARTIFACTS_DOCKERFILE) -t $(CUDA_ARTIFACTS_IMAGE) .
	-$(DOCKER) rm -f $(CUDA_ARTIFACTS_CONTAINER) >/dev/null 2>&1
	$(DOCKER) run --name $(CUDA_ARTIFACTS_CONTAINER) $(CUDA_ARTIFACTS_IMAGE)
	mkdir -p $(CUDA_ARTIFACTS_PREBUILT_DIR)
	$(DOCKER) cp $(CUDA_ARTIFACTS_CONTAINER):/src/.slang-artifacts/cuda/$(CUDA_TARGET)/. $(CUDA_ARTIFACTS_PREBUILT_DIR)/
	find $(CUDA_ARTIFACTS_PREBUILT_DIR) -maxdepth 1 -type f ! -name '*.cubin' -delete
	-chown -R "$$(id -u):$$(id -g)" $(CUDA_ARTIFACTS_PREBUILT_DIR) 2>/dev/null
	$(DOCKER) rm -f $(CUDA_ARTIFACTS_CONTAINER) >/dev/null 2>&1

# Idempotent: installs flatpak, flatpak-builder, the flathub remote, and the
# org.gnome Platform/Sdk pair if missing, skips anything already present.
# Package names are assumed to be pacman's (this repo's target machine is
# CachyOS/Arch per CLAUDE.md) -- override PACMAN if that's wrong for your
# host.
PACMAN ?= sudo pacman
flatpak-sdk:
	@command -v $(FLATPAK) >/dev/null 2>&1 || { echo "Installing flatpak..."; $(PACMAN) -S --noconfirm flatpak; }
	@command -v $(FLATPAK_BUILDER) >/dev/null 2>&1 || { echo "Installing flatpak-builder..."; $(PACMAN) -S --noconfirm flatpak-builder; }
	@$(FLATPAK) remote-list | grep -q '^flathub' || $(FLATPAK) remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
	@$(FLATPAK) info org.gnome.Platform//$(FLATPAK_RUNTIME_VERSION) >/dev/null 2>&1 || $(FLATPAK) install -y flathub org.gnome.Platform//$(FLATPAK_RUNTIME_VERSION)
	@$(FLATPAK) info org.gnome.Sdk//$(FLATPAK_RUNTIME_VERSION) >/dev/null 2>&1 || $(FLATPAK) install -y flathub org.gnome.Sdk//$(FLATPAK_RUNTIME_VERSION)

# Only the submodules the flatpak build path actually needs (slang for
# cuda-artifacts and the M6 slang module, optix-dev/vtracer as
# shrimply-render-cuda/-video-core path deps). external/manim is out of
# scope -- see TODO.md. slang needs --recursive: its own build (glslang,
# spirv-tools, etc., see M6) pulls in nested submodules under
# external/slang/external/ that a plain (non-recursive) init leaves empty.
flatpak-submodules:
	@test -e external/slang/external/glslang/CMakeLists.txt || git submodule update --init --recursive external/slang
	@test -e external/optix-dev/README.md || git submodule update --init external/optix-dev
	@test -e external/vtracer/Cargo.toml || git submodule update --init external/vtracer

# Skips the (heavy, Docker-based) rebuild if cubins are already vendored --
# see M4 in TODO.md. Delete crates/render-cuda/prebuilt/$(CUDA_TARGET)/ first
# to force regeneration (e.g. after a shader change).
flatpak-cuda-vendor: flatpak-submodules
	@if [ -z "$$(ls -A $(CUDA_ARTIFACTS_PREBUILT_DIR) 2>/dev/null)" ]; then \
		$(MAKE) cuda-artifacts-image; \
	else \
		echo "CUDA cubins already vendored at $(CUDA_ARTIFACTS_PREBUILT_DIR)/, skipping"; \
	fi

flatpak-skeleton: flatpak-sdk metainfo-check desktop-file-check
	$(FLATPAK_BUILDER) --force-clean $(FLATPAK_BUILD_DIR) $(FLATPAK_MANIFEST)

# M5 (see TODO.md): installs the Rust nightly Sdk extension the flatpak build
# uses for its toolchain. Idempotent like flatpak-sdk.
flatpak-rust-sdk:
	@$(FLATPAK) info org.freedesktop.Sdk.Extension.rust-nightly//$(FLATPAK_SDK_EXTENSION_BRANCH) >/dev/null 2>&1 || $(FLATPAK) install -y flathub org.freedesktop.Sdk.Extension.rust-nightly//$(FLATPAK_SDK_EXTENSION_BRANCH)

# M7: installs the LLVM Sdk extension (bindgen/opencv-binding-generator need
# libclang.so + the clang binary, org.gnome.Sdk//50 has neither). Idempotent.
flatpak-llvm-sdk:
	@$(FLATPAK) info org.freedesktop.Sdk.Extension.llvm22//$(FLATPAK_SDK_EXTENSION_BRANCH) >/dev/null 2>&1 || $(FLATPAK) install -y flathub org.freedesktop.Sdk.Extension.llvm22//$(FLATPAK_SDK_EXTENSION_BRANCH)

# In-sandbox only -- has no business running on the host, which has no
# `/usr/lib/sdk/rust-nightly` and normally drives cargo through rustup
# instead (see CARGO's definition above). A manifest build-command sources
# the extension's enable.sh for PATH, then invokes this with `CARGO=cargo` to
# override the rustup-wrapped default, e.g.:
#   source /usr/lib/sdk/rust-nightly/enable.sh && make flatpak-rust-check CARGO=cargo
# Checks a single leaf crate (shrimply-math-core: no native deps, nothing
# else in the workspace depends on it) rather than the whole workspace --
# M6's native deps (ffmpeg/poppler/opencv/...) don't exist in the sandbox
# yet, see TODO.md M5.
flatpak-rust-check:
	rustc --version
	$(CARGO) --version
	$(CARGO) check -p shrimply-math-core

# Reproduces the flatpak packaging work done through M6 (see TODO.md) from a
# fresh clone in one command: installs flatpak-builder + the SDK/runtime pair
# + the rust-nightly and llvm22 Sdk extensions, initializes the submodules
# the build needs, vendors the CUDA cubins if not already present (needs
# Docker), then runs flatpak-skeleton -- which, now that M6's 6 dependency
# modules and M7's app module are in the manifest, is no longer a quick
# metadata-only build: it's a real, possibly hour-plus build of
# rubberband/ffmpeg/poppler/vte/opencv/slang/shrimply from source.
# `--share=network` is still on, so this also needs network access for the
# module sources. See TODO.md's M7 section for the ~/.cache/
# shrimply-flatpak-cargo-cache CI caching notes.
flatpak-bootstrap: flatpak-sdk flatpak-rust-sdk flatpak-llvm-sdk flatpak-cuda-vendor flatpak-skeleton
	@echo "Flatpak packaging reproduced through M7."

# M7: `dist` was deliberately left unimplemented (listed in .PHONY with no
# recipe) until a real app module landed -- see TODO.md's "Decided" section.
# It has now, so this is real: produces a single-file .flatpak bundle, the
# distribution model TODO.md's Target section actually calls for (self-hosted
# repo or single-file bundle, NOT Flathub -- blocked by the NVIDIA
# redistributables and the pinned Rust nightly). `dist-image` was the other
# historical option floated for this and is deleted, not implemented, per
# that same note.
FLATPAK_APP_ID ?= dev.shrimply.Shrimply
FLATPAK_REPO_DIR ?= repo
FLATPAK_BUNDLE ?= $(FLATPAK_APP_ID).flatpak
dist: flatpak-sdk flatpak-rust-sdk flatpak-llvm-sdk flatpak-cuda-vendor metainfo-check desktop-file-check
	$(FLATPAK_BUILDER) --repo=$(FLATPAK_REPO_DIR) --force-clean $(FLATPAK_BUILD_DIR) $(FLATPAK_MANIFEST)
	$(FLATPAK) build-bundle $(FLATPAK_REPO_DIR) $(FLATPAK_BUNDLE) $(FLATPAK_APP_ID)
	@echo "Flatpak bundle: $(FLATPAK_BUNDLE)"

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

APPKIT_BUILD_ENV = $(SLANG_LIBRARY_ENV) RUSTFLAGS="-C prefer-dynamic -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)" LIBRARY_PATH="$$(brew --prefix)/lib" PKG_CONFIG="$$(brew --prefix pkgconf)/bin/pkg-config" CLANG_PATH="$$(brew --prefix llvm@18)/bin/clang" LIBCLANG_PATH="$$(brew --prefix llvm@18)/lib" SLANG_SOURCE_DIR=$(SLANG_SOURCE_DIR) SLANG_BUILD_DIR=$(SLANG_BUILD_DIR)

.PHONY: appkit-build appkit-check appkit-lint appkit-components-check appkit-components-showcase
$(APPKIT_ICON): $(APPKIT_ICON_SOURCE)
	$(RSVG_CONVERT) --width $(APPKIT_ICON_SIZE) --height $(APPKIT_ICON_SIZE) $< --output $@

appkit-build: $(APPKIT_ICON)
	@test "$$(uname -s)" = Darwin || { echo "dev-mac requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) build -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins

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

check: native-deps qt-native-deps cuda-artifacts fmt source-size-check cargo-check lint server-python-check manim-python-check docs-check metainfo-check desktop-file-check

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
	cd crates/manim/manim-parser/python && uv run --python 3.14 python -m shrimply_manim $(ARGS)

manim-python-check:
	uv run --python 3.14 --project crates/manim/manim-parser/python pyrefly check --python-version 3.14 --search-path external/manim crates/manim/manim-parser/python/shrimply_manim

manim-visual-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-wgpu --test visual_parity -- --ignored --nocapture

manim-parameter-check: native-deps
	$(DEV_BUILD_ENV) $(CARGO) test -p shrimply-manim-parser --test two_pass_parameters -- --ignored --nocapture

metainfo-check:
	@command -v $(APPSTREAMCLI) >/dev/null 2>&1 || { echo "Installing appstream..."; $(PACMAN) -S --noconfirm appstream; }
	$(APPSTREAMCLI) validate assets/dev.shrimply.Shrimply.metainfo.xml

desktop-file-check:
	@command -v $(DESKTOP_FILE_VALIDATE) >/dev/null 2>&1 || { echo "Installing desktop-file-utils..."; $(PACMAN) -S --noconfirm desktop-file-utils; }
	$(DESKTOP_FILE_VALIDATE) assets/dev.shrimply.Shrimply.desktop

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

install: release desktop-icon
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
	@if test -z "$(DESTDIR)"; then \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
	fi
	@echo "Installed $(APP_NAME) under $(DESTDIR)$(PREFIX)"

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
