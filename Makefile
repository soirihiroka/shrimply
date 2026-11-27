.DEFAULT_GOAL := build

HOST_OS := $(if $(filter Windows_NT,$(OS)),Windows_NT,$(shell uname -s))
CARGO ?= cargo
CARGO_TARGET_DIR ?= target
CUDA_HOME ?= $(if $(filter Windows_NT,$(HOST_OS)),$(CUDA_PATH),/usr/local/cuda)
CUDA_TOOLKIT_PATH ?= $(CUDA_HOME)
CUDA_TARGET ?= sm_86
CUDA_IMAGE_FORMAT ?= cubin
CUDA_PTX_TARGET ?= compute_50
CUDA_HOST_CXX ?= g++-15
CUDA_ALLOW_UNSUPPORTED_COMPILER ?=
ifeq ($(HOST_OS),Windows_NT)
WINDOWS_DEPS_ROOT ?= $(CURDIR)/target/windows-deps
VCPKG_ROOT ?= $(WINDOWS_DEPS_ROOT)/vcpkg
VCPKG_INSTALLED_DIR ?= $(WINDOWS_DEPS_ROOT)/installed
FFMPEG_DIR := $(VCPKG_INSTALLED_DIR)/x64-windows
endif
ifeq ($(HOST_OS),Linux)
CUDA_DEV_TARGETS := build dev dev-qt qt-build
else ifeq ($(HOST_OS),Windows_NT)
CUDA_DEV_TARGETS := build dev windows-build windows-dev
endif
ifneq ($(filter $(CUDA_DEV_TARGETS),$(or $(MAKECMDGOALS),$(.DEFAULT_GOAL))),)
CUDA_NATIVE_TARGET := $(shell uv run --no-project python packaging/cuda-target.py)
ifeq ($(strip $(CUDA_NATIVE_TARGET)),)
$(error Could not detect the local CUDA architecture for the development build)
endif
$(CUDA_DEV_TARGETS): override CUDA_IMAGE_FORMAT = cubin
$(CUDA_DEV_TARGETS): override CUDA_TARGET := $(CUDA_NATIVE_TARGET)
endif
OPTIX_ROOT ?= $(CURDIR)/external/optix-dev
DNF ?= sudo dnf
INSTALL ?= install
PKG_CONFIG ?= $(if $(filter Windows_NT,$(HOST_OS)),pkg-config,/usr/bin/pkg-config)
PKG_CONFIG_PATH ?= /usr/lib64/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig
ifeq ($(HOST_OS),Linux)
LINUX_FFMPEG_CACHE := $(abspath $(CARGO_TARGET_DIR)/linux-ffmpeg)
LINUX_FFMPEG_PREFIX = $(or $(SHRIMPLY_FFMPEG_PREFIX),$(LINUX_FFMPEG_CACHE)/install)
# Use the resolved source-specific prefix so a gitlink update invalidates Rust bindings.
FFMPEG_PREFIX = $(or $(realpath $(LINUX_FFMPEG_PREFIX)),$(LINUX_FFMPEG_PREFIX))
FFMPEG_RUNTIME_ENV = PATH="$(FFMPEG_PREFIX)/bin:$(CUDA_HOME)/bin:$(PATH)" LD_LIBRARY_PATH="$(FFMPEG_PREFIX)/lib:/run/host/usr/local/libclang-deps:$${LD_LIBRARY_PATH}"
HOST_MEDIA_ENV = $(FFMPEG_RUNTIME_ENV) FFMPEG_DIR="$(FFMPEG_PREFIX)" SHRIMPLY_FFMPEG_PREFIX="$(FFMPEG_PREFIX)" \
    PKG_CONFIG_PATH="$(FFMPEG_PREFIX)/lib/pkgconfig:$(PKG_CONFIG_PATH)" LIBRARY_PATH="$(FFMPEG_PREFIX)/lib:$${LIBRARY_PATH}"
else ifeq ($(HOST_OS),Darwin)
FFMPEG_PREFIX = $(APPKIT_FFMPEG)/install
FFMPEG_RUNTIME_ENV = PATH="$(FFMPEG_PREFIX)/bin:$(PATH)" FFMPEG_DIR="$(FFMPEG_PREFIX)"
HOST_MEDIA_ENV = $(FFMPEG_RUNTIME_ENV) PKG_CONFIG_PATH="$(APPKIT_FFMPEG)/install/lib/pkgconfig:$$PKG_CONFIG_PATH"
else ifeq ($(HOST_OS),Windows_NT)
FFMPEG_PREFIX = $(FFMPEG_DIR)
FFMPEG_RUNTIME_ENV = FFMPEG_DIR="$(FFMPEG_PREFIX)" PATH="$(FFMPEG_PREFIX)/bin:$(PATH)"
HOST_MEDIA_ENV = $(FFMPEG_RUNTIME_ENV) INCLUDE="$(FFMPEG_DIR)/include;$$INCLUDE" LIB="$(FFMPEG_DIR)/lib;$$LIB"
endif
QT_QMAKE ?= $(if $(filter Windows_NT,$(HOST_OS)),qmake.exe,qmake6)
BUILD_ENV = CUDA_HOME="$(CUDA_HOME)" CUDA_TOOLKIT_PATH="$(CUDA_TOOLKIT_PATH)" CUDA_IMAGE_FORMAT=$(CUDA_IMAGE_FORMAT) CUDA_TARGET=$(CUDA_TARGET) CUDA_PTX_TARGET=$(CUDA_PTX_TARGET) CUDA_HOST_CXX=$(CUDA_HOST_CXX) CUDA_ALLOW_UNSUPPORTED_COMPILER=$(CUDA_ALLOW_UNSUPPORTED_COMPILER) PKG_CONFIG="$(PKG_CONFIG)" OPTIX_ROOT="$(OPTIX_ROOT)"
ifeq ($(HOST_OS),Linux)
# Native dependencies can add /usr/lib64 ahead of ffmpeg-sys's link search path.
# Give the project ABI precedence in the final linker command for both profiles.
FFMPEG_LINK_FLAG = -Zpre-link-arg=-L$(FFMPEG_PREFIX)/lib
BUILD_ENV += $(HOST_MEDIA_ENV) RUSTFLAGS="$(FFMPEG_LINK_FLAG) $(or $(RUSTFLAGS),-C link-arg=-fuse-ld=lld)"
else
BUILD_ENV += PKG_CONFIG_PATH="$(PKG_CONFIG_PATH)"
ifneq ($(HOST_OS),Windows_NT)
BUILD_ENV += PATH="$(CUDA_HOME)/bin:$(PATH)"
BUILD_ENV += LD_LIBRARY_PATH="/run/host/usr/local/libclang-deps:$${LD_LIBRARY_PATH}"
endif
endif
ifneq ($(HOST_OS),Windows_NT)
BUILD_ENV += BINDGEN_EXTRA_CLANG_ARGS="$(if $(wildcard /run/host/usr/lib/llvm-18/lib/clang/18/include),-isystem /run/host/usr/lib/llvm-18/lib/clang/18/include)"
endif
RUST_LIBDIR = $(shell rustc --print target-libdir)
ifeq ($(HOST_OS),Windows_NT)
DEV_RUSTFLAGS ?=
else
DEV_RUSTFLAGS ?= -C prefer-dynamic -C link-arg=-fuse-ld=lld -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)
endif
DEV_BUILD_ENV = $(BUILD_ENV) RUSTFLAGS="$(strip $(FFMPEG_LINK_FLAG) $(DEV_RUSTFLAGS))"

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
GTK_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-gtk
QT_COMPONENTS_DEMO_PACKAGE := shrimply-components-demo-qt
QT_TARGET_DIR ?= $(if $(filter Linux,$(HOST_OS)),$(CARGO_TARGET_DIR)/qt,$(CARGO_TARGET_DIR))
BINARY_PACKAGES := \
	$(EDITOR_PACKAGE) \
	$(QT_EDITOR_PACKAGE) \
	$(LAUNCHER_PACKAGE) \
	$(QT_LAUNCHER_PACKAGE) \
	$(APPKIT_LAUNCHER_PACKAGE) \
	$(APPKIT_EDITOR_PACKAGE) \
	$(APPKIT_COMPONENTS_DEMO_PACKAGE) \
	$(GTK_COMPONENTS_DEMO_PACKAGE) \
	$(QT_COMPONENTS_DEMO_PACKAGE)
RUST_LOG ?= info,shrimply=debug,shrimply_editor=debug,shrimply_launcher=debug,shrimply_launcher_qt=debug,shrimply_timeline_gtk=debug
DEV_LOG ?= target/$(BIN_NAME)-dev.log
QT_DEV_LOG ?= $(QT_TARGET_DIR)/$(BIN_NAME)-dev.log
CRASH_CORE ?= target/$(EDITOR_BIN_NAME).core
CRASH_STACK ?= target/$(EDITOR_BIN_NAME).stack
CRASH_PROFILE ?= debug
CRASH_SINCE ?= -1 day

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
APPLICATIONSDIR ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/apps
MIMEDIR ?= $(DATADIR)/mime
MIME_ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/mimetypes
PROJECT_MIME := assets/dev.shrimply.Shrimply-mime.xml
PROJECT_ICON := assets/icons/dev.shrimply.Shrimply-project.svg
DESKTOP_FILE := assets/dev.shrimply.Shrimply.desktop
APP_ICON := assets/icons/dev.shrimply.Shrimply.svg
APPKIT_ICON_SOURCE := packaging/macos/Shrimply.icon
APPKIT_ICON_DIR := $(CARGO_TARGET_DIR)/macos-icon
APPKIT_APP := $(CARGO_TARGET_DIR)/debug/Shrimply.app
APPKIT_DEPLOYMENT_TARGET ?= 15.0
RSVG_CONVERT ?= rsvg-convert
LIP_SYNC_LICENSE_DIR := $(DATADIR)/licenses/shrimply
MANIM_WORKER_RESOURCE_DIR := $(PREFIX)/share/shrimply/crates/media/visual/manim/manim-bridge/python
MANIM_SOURCE_RESOURCE_DIR := $(PREFIX)/share/shrimply/external/manim

FEDORA_PACKAGES := \
	rust \
	cargo \
	clang \
	clang-devel \
	cmake \
	gcc-c++ \
	gcc15-c++ \
	lld \
	ninja-build \
	openssl-devel \
	pkgconf-pkg-config \
	gobject-introspection-devel \
	patchelf \
	nasm \
	nv-codec-headers \
	libva-devel \
	libdrm-devel \
	vulkan-headers \
	spirv-headers-devel \
	gnutls-devel \
	opus-devel \
	lame-devel \
	libvorbis-devel \
	libdav1d-devel \
	x264-devel \
	x265-devel \
	fdk-aac-devel \
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
	qt6-qtdeclarative-devel \
	spirv-tools

ifeq ($(HOST_OS),Linux)
linux-ffmpeg:
ifeq ($(strip $(SHRIMPLY_FFMPEG_PREFIX)),)
	PKG_CONFIG="$(PKG_CONFIG)" PKG_CONFIG_PATH="$(PKG_CONFIG_PATH)" uv run --no-project --python 3.13 python packaging/linux-ffmpeg.py "$(LINUX_FFMPEG_CACHE)"
else
	@test -f "$(FFMPEG_PREFIX)/include/libavcodec/avcodec.h" && test -f "$(FFMPEG_PREFIX)/lib/libavcodec.so" || { echo "Missing packaged FFmpeg under $(FFMPEG_PREFIX)" >&2; exit 1; }
	@$(HOST_MEDIA_ENV) "$(FFMPEG_PREFIX)/bin/ffmpeg" -version | awk -v required="$$(cat external/ffmpeg/RELEASE)" 'NR == 1 { actual = $$3; sub(/^n/, "", actual) } END { if (actual != required || !required) { print "Expected project FFmpeg " required ", found " actual > "/dev/stderr"; exit 1 } }'
endif
native-deps sanim-check: linux-ffmpeg
endif
native-deps:
	@$(PKG_CONFIG) --exists libpipewire-0.3 || { echo "Missing PipeWire development files (pkg-config: libpipewire-0.3)" >&2; exit 1; }
	@$(PKG_CONFIG) --exists poppler-glib || { echo "Missing Poppler GLib development files (pkg-config: poppler-glib)" >&2; exit 1; }

windows-native-deps: qt-native-deps
	@test "$(HOST_OS)" = Windows_NT || { echo "Windows targets require native Windows" >&2; exit 1; }
	@command -v cl.exe >/dev/null 2>&1 || { echo "Missing MSVC compiler (cl.exe)" >&2; exit 1; }
	@command -v nvcc.exe >/dev/null 2>&1 || { echo "Missing CUDA compiler (nvcc.exe)" >&2; exit 1; }
	@powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File packaging/windows/verify-ffmpeg.ps1 -FfmpegRoot "$(FFMPEG_DIR)" -DevelopmentFilesOnly
	@$(PKG_CONFIG) --exists poppler-glib pango pangocairo || { echo "Missing vcpkg application libraries" >&2; exit 1; }

qt-native-deps:
	@command -v $(QT_QMAKE) >/dev/null 2>&1 || { echo "Missing Qt 6 qmake ($(QT_QMAKE))" >&2; exit 1; }
	@version="$$($(QT_QMAKE) -query QT_VERSION)"; case "$$version" in 6.*) echo "Using Qt $$version via $(QT_QMAKE)" ;; *) echo "$(QT_QMAKE) selected unsupported Qt $$version; Qt 6 is required" >&2; exit 1 ;; esac
	@if test "$(HOST_OS)" != Windows_NT; then $(PKG_CONFIG) --exists Qt6Core Qt6Gui Qt6Qml Qt6Quick Qt6QuickControls2 Qt6OpenGL || { echo "Missing Qt 6 Quick/OpenGL development files" >&2; exit 1; }; fi

cuda-target-check:
	@case "$(HOST_OS)" in Linux|Windows_NT) ;; (*) echo "CUDA kernels require Linux or Windows" >&2; exit 1 ;; esac
	@case "$(CUDA_IMAGE_FORMAT)" in \
		(cubin) case "$(CUDA_TARGET)" in sm_*) ;; (*) echo "CUDA_TARGET=$(CUDA_TARGET) must be a physical SM architecture" >&2; exit 1 ;; esac ;; \
		(ptx) case "$(CUDA_PTX_TARGET)" in compute_*) ;; (*) echo "CUDA_PTX_TARGET=$(CUDA_PTX_TARGET) must be a virtual compute architecture" >&2; exit 1 ;; esac ;; \
		(*) echo "CUDA_IMAGE_FORMAT=$(CUDA_IMAGE_FORMAT) is unsupported; expected cubin or ptx" >&2; exit 1 ;; \
	esac

ifeq ($(HOST_OS),Linux)
dev: build
dev-qt: qt-build
dev: private DEV_TARGET_DIR = $(CARGO_TARGET_DIR)
dev-qt: private DEV_TARGET_DIR = $(QT_TARGET_DIR)
dev-qt: private DEV_LOG = $(QT_DEV_LOG)
dev dev-qt: SHELL := /bin/bash
dev dev-qt:
	@started="$$(date --iso-8601=seconds)"; \
	$(BUILD_ENV) RUST_LOG=$(RUST_LOG) "$(DEV_TARGET_DIR)/debug/$(BIN_NAME)" 2>&1 \
		| tee >(sed -E 's/\x1B\[[0-9;]*[[:alpha:]]//g' > "$(DEV_LOG)"); \
	status=$${PIPESTATUS[0]}; \
	if [[ $$status -ne 0 ]]; then \
		$(MAKE) crash-report CRASH_SINCE="$$started" CRASH_PROFILE=debug || true; \
		echo "Debug trace: $(DEV_LOG)"; \
	fi; \
	exit $$status
endif

APPKIT_FFMPEG := $(abspath $(CARGO_TARGET_DIR)/macos-ffmpeg)
appkit-ffmpeg:
	MACOSX_DEPLOYMENT_TARGET=$(APPKIT_DEPLOYMENT_TARGET) uv run --no-project --python 3.13 python packaging/macos-ffmpeg.py "$(APPKIT_FFMPEG)"

APPKIT_BUILD_ENV = $(HOST_MEDIA_ENV) RUSTFLAGS="-C prefer-dynamic -C link-arg=-Wl,-rpath,$(RUST_LIBDIR)" LIBRARY_PATH="$(APPKIT_FFMPEG)/install/lib:$$(brew --prefix)/lib" PKG_CONFIG="$$(brew --prefix pkgconf)/bin/pkg-config" CLANG_PATH="$$(brew --prefix llvm@22)/bin/clang" LIBCLANG_PATH="$$(brew --prefix llvm@22)/lib"

appkit-icon:
	mkdir -p "$(APPKIT_ICON_DIR)"
	xcrun actool "$(APPKIT_ICON_SOURCE)" --compile "$(APPKIT_ICON_DIR)" \
		--app-icon Shrimply --platform macosx --target-device mac \
		--minimum-deployment-target $(APPKIT_DEPLOYMENT_TARGET) \
		--output-partial-info-plist "$(APPKIT_ICON_DIR)/Info.plist" \
		--output-format human-readable-text --warnings --notices

ifeq ($(HOST_OS),Darwin)
build: appkit-ffmpeg appkit-icon
	$(APPKIT_BUILD_ENV) $(CARGO) build -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins
	mkdir -p "$(APPKIT_APP)/Contents/MacOS" "$(APPKIT_APP)/Contents/Resources/licenses"
	cp $(CARGO_TARGET_DIR)/debug/$(BIN_NAME) "$(APPKIT_APP)/Contents/MacOS/$(BIN_NAME)"
	cp "$(APPKIT_FFMPEG)/install/bin/ffmpeg" "$(APPKIT_APP)/Contents/MacOS/shrimply-ffmpeg"
	cp packaging/macos/Info.plist "$(APPKIT_APP)/Contents/Info.plist"
	cp "$(APPKIT_ICON_DIR)/Assets.car" "$(APPKIT_ICON_DIR)/Shrimply.icns" "$(APPKIT_APP)/Contents/Resources/"
	cp vendor/vision/LICENSE "$(APPKIT_APP)/Contents/Resources/licenses/OpenCV-Vision.txt"
	cp vendor/rubberband/COPYING "$(APPKIT_APP)/Contents/Resources/licenses/Rubber-Band.txt"
	cp crates/math/color/src/css/LICENSE "$(APPKIT_APP)/Contents/Resources/licenses/GTK-Pango-Color.txt"
	cp crates/math/color/src/css/NOTICE "$(APPKIT_APP)/Contents/Resources/licenses/GTK-Pango-Color-NOTICE.txt"

dev: dev-mac
dev-mac: build
	$(HOST_MEDIA_ENV) RUST_LOG=$(RUST_LOG) "$(APPKIT_APP)/Contents/MacOS/$(BIN_NAME)"
release: appkit-release
endif

appkit-release: appkit-ffmpeg appkit-icon
	@test "$$(uname -s)" = Darwin || { echo "AppKit release requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) RUSTFLAGS="" CARGO_TERM_COLOR=always MACOSX_DEPLOYMENT_TARGET=$(APPKIT_DEPLOYMENT_TARGET) $(CARGO) build --release -p $(APPKIT_LAUNCHER_PACKAGE) -p $(APPKIT_EDITOR_PACKAGE) --bins

appkit-check: appkit-ffmpeg
	@test "$$(uname -s)" = Darwin || { echo "AppKit lint requires macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) clippy -p $(APPKIT_EDITOR_PACKAGE) -p $(APPKIT_LAUNCHER_PACKAGE) -p $(FRAMEGRAPH_CORE_PACKAGE) -p $(APPKIT_COMPONENT_METAL_PACKAGE) -p $(APPKIT_COMPONENTS_PACKAGE) -p $(APPKIT_COMPONENTS_DEMO_PACKAGE) --all-targets -- -D warnings

appkit-components-showcase: appkit-ffmpeg
	@test "$$(uname -s)" = Darwin || { echo "AppKit components require macOS" >&2; exit 1; }
	$(APPKIT_BUILD_ENV) $(CARGO) run -p $(APPKIT_COMPONENTS_DEMO_PACKAGE)

qt-build qt-release: native-deps qt-native-deps cuda-target-check
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) build --target-dir "$(QT_TARGET_DIR)" $(if $(filter qt-release,$@),--release) -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) --bins

dev-server:
	uv run --project server --locked server/src/main.py

docs:
	uv run --project docs --locked sphinx-build docs/source docs/build

docs-check:
	uv run --project docs --locked sphinx-build -W --keep-going docs/source docs/build

ifeq ($(HOST_OS),Linux)
build release: native-deps cuda-target-check
	$(if $(filter release,$@),$(BUILD_ENV),$(DEV_BUILD_ENV)) $(CARGO) build $(if $(filter release,$@),--release) -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) --bins

check: fmt-check lint sanim-check project-server-check server-python-check manim-python-check docs-check
else ifeq ($(HOST_OS),Darwin)
check: appkit-check fmt-check
else ifeq ($(HOST_OS),Windows_NT)
build: windows-build
dev: windows-dev
release: windows-package
check: windows-check
else
check:
	@echo "make check supports Linux, macOS, and Windows; unsupported platform: $(HOST_OS)" >&2
	@exit 1
endif

ifeq ($(HOST_OS),Windows_NT)
windows-build: windows-native-deps cuda-target-check
	$(BUILD_ENV) QMAKE="$(QT_QMAKE)" CARGO_TERM_COLOR=always $(CARGO) build --target-dir "$(QT_TARGET_DIR)" -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) --bins

windows-dev: windows-build
	$(BUILD_ENV) $(HOST_MEDIA_ENV) RUST_LOG="$(RUST_LOG)" $(QT_TARGET_DIR)/debug/$(BIN_NAME).exe
else
windows-build windows-dev:
	@echo "$@ requires Windows; current host: $(HOST_OS)" >&2
	@exit 1
endif

windows-check: override CUDA_IMAGE_FORMAT = ptx
windows-check: override CUDA_PTX_TARGET = compute_50
windows-check: windows-native-deps fmt-check cuda-target-check
	$(BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) clippy -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) --bins -- -D warnings

ifeq ($(HOST_OS),Windows_NT)
# GNU Make tracks Windows shell mode globally, so choose it once per invocation.
ifneq ($(filter release windows-package,$(MAKECMDGOALS)),)
SHELL := cmd.exe
.SHELLFLAGS := /d /c
else
SHELL := bash.exe
endif
endif

windows-package: override CUDA_IMAGE_FORMAT = ptx
# Maxwell baseline; PTX is JIT-compiled for newer GPUs, including Blackwell.
windows-package: override CUDA_PTX_TARGET = compute_50
windows-package:
	powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "packaging/windows/windows-release.ps1" -VcpkgRoot "$(VCPKG_ROOT)" -InstalledDir "$(VCPKG_INSTALLED_DIR)" -Qmake "$(QT_QMAKE)" -CargoTargetDir "$(CARGO_TARGET_DIR)" -CudaHome "$(CUDA_HOME)" -CudaImageFormat "$(CUDA_IMAGE_FORMAT)" -CudaTarget "$(CUDA_TARGET)" -CudaPtxTarget "$(CUDA_PTX_TARGET)" -CudaHostCxx "$(CUDA_HOST_CXX)" -CudaAllowUnsupportedCompiler "$(CUDA_ALLOW_UNSUPPORTED_COMPILER)" -OptixRoot "$(OPTIX_ROOT)" -QtEditorPackage "$(QT_EDITOR_PACKAGE)" -QtLauncherPackage "$(QT_LAUNCHER_PACKAGE)"

	powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File packaging/windows/package.ps1

gtk-components-showcase: native-deps
	$(DEV_BUILD_ENV) $(CARGO) run -p $(GTK_COMPONENTS_DEMO_PACKAGE)

qt-components-showcase: qt-native-deps
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) run -p $(QT_COMPONENTS_DEMO_PACKAGE)

sanim-check:
	$(FFMPEG_RUNTIME_ENV) $(MAKE) -C sanim check CARGO="$(CARGO)"

project-server:
	$(CARGO) build -p shrimply-project-server

PROJECT_SERVER_LOCK ?= --locked
project-server-release:
	$(CARGO) build $(PROJECT_SERVER_LOCK) --release -p shrimply-project-server

project-server-check:
	$(CARGO) clippy --locked -p shrimply-project-server --all-targets -- -D warnings

server-python-check:
	uv run --directory server --locked pyrefly check

manim:
	uv run --directory crates/media/visual/manim/manim-bridge/python --python 3.14 python -m shrimply_manim $(ARGS)

manim-python-check:
	uv run --python 3.14 --project crates/media/visual/manim/manim-bridge/python pyrefly check --python-version 3.14 --search-path external/manim crates/media/visual/manim/manim-bridge/python/shrimply_manim

fmt fmt-check:
	$(CARGO) fmt $(if $(filter fmt-check,$@),--check)

lint: native-deps qt-native-deps cuda-target-check
	$(DEV_BUILD_ENV) $(CARGO) clippy -p $(EDITOR_PACKAGE) -p $(LAUNCHER_PACKAGE) --bins -- -D warnings
	$(DEV_BUILD_ENV) QMAKE=$(QT_QMAKE) $(CARGO) clippy -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE) --bins -- -D warnings

test: $(if $(filter Linux,$(HOST_OS)),native-deps cuda-target-check)
	$(DEV_BUILD_ENV) $(CARGO) test $(ARGS)

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

clean:
	$(CARGO) clean $(foreach package,$(BINARY_PACKAGES),--package $(package))
ifneq ($(QT_TARGET_DIR),$(CARGO_TARGET_DIR))
	$(CARGO) clean --target-dir "$(QT_TARGET_DIR)" -p $(QT_EDITOR_PACKAGE) -p $(QT_LAUNCHER_PACKAGE)
endif

deps-fedora:
	$(DNF) install $(FEDORA_PACKAGES)

ifeq ($(HOST_OS),Linux)
install: release
	sh packaging/linux/install-manim-worker.sh "$(DESTDIR)$(PREFIX)"
	$(INSTALL) -Dm644 $(APP_ICON) "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	$(INSTALL) -Dm644 $(PROJECT_ICON) "$(DESTDIR)$(MIME_ICONDIR)/dev.shrimply.Shrimply-project.svg"
	$(INSTALL) -Dm644 $(PROJECT_MIME) "$(DESTDIR)$(MIMEDIR)/packages/dev.shrimply.Shrimply-mime.xml"
	$(INSTALL) -Dm755 $(CARGO_TARGET_DIR)/release/$(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	$(INSTALL) -Dm755 $(CARGO_TARGET_DIR)/release/$(EDITOR_BIN_NAME) "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
ifneq ($(FFMPEG_PREFIX),$(abspath $(DESTDIR)$(PREFIX)))
	sh packaging/linux/install-ffmpeg.sh "$(FFMPEG_PREFIX)" "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(PREFIX)/lib/shrimply/ffmpeg" "$(BIN_NAME)" "$(EDITOR_BIN_NAME)"
endif
	$(INSTALL) -Dm644 vendor/pocketsphinx/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	$(INSTALL) -Dm644 vendor/pocketsphinx/MODEL-LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	$(INSTALL) -Dm644 vendor/rhubarb-lip-sync/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	$(INSTALL) -Dm644 vendor/vision/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/OpenCV-Vision.txt"
	$(INSTALL) -Dm644 vendor/rubberband/COPYING "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rubber-Band.txt"
	$(INSTALL) -Dm644 crates/math/color/src/css/LICENSE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/GTK-Pango-Color.txt"
	$(INSTALL) -Dm644 crates/math/color/src/css/NOTICE "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/GTK-Pango-Color-NOTICE.txt"
	sed -e 's|^Exec=.*|Exec="$(BINDIR)/$(BIN_NAME)" %f|' -e 's|^TryExec=.*|TryExec=$(BINDIR)/$(BIN_NAME)|' $(DESKTOP_FILE) | $(INSTALL) -Dm644 /dev/stdin "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	@if test -z "$(DESTDIR)"; then \
		update-mime-database "$(MIMEDIR)"; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor" >/dev/null || true; \
		command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(APPLICATIONSDIR)" >/dev/null || true; \
	fi
	@echo "Installed $(APP_NAME) under $(DESTDIR)$(PREFIX)"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/shrimply-ffmpeg"
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(EDITOR_BIN_NAME)"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-code.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/PocketSphinx-model.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rhubarb-Lip-Sync.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/OpenCV-Vision.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/Rubber-Band.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/GTK-Pango-Color.txt"
	rm -f "$(DESTDIR)$(LIP_SYNC_LICENSE_DIR)/GTK-Pango-Color-NOTICE.txt"
	rm -f "$(DESTDIR)$(APPLICATIONSDIR)/dev.shrimply.Shrimply.desktop"
	rm -rf "$(DESTDIR)$(MANIM_WORKER_RESOURCE_DIR)" "$(DESTDIR)$(MANIM_SOURCE_RESOURCE_DIR)" "$(DESTDIR)$(PREFIX)/lib/shrimply/ffmpeg"
	rm -f "$(DESTDIR)$(MIMEDIR)/packages/dev.shrimply.Shrimply-mime.xml" \
		"$(DESTDIR)$(MIME_ICONDIR)/dev.shrimply.Shrimply-project.svg" "$(DESTDIR)$(ICONDIR)/dev.shrimply.Shrimply.svg"
	@if test -z "$(DESTDIR)"; then \
		if test -d "$(MIMEDIR)"; then update-mime-database "$(MIMEDIR)"; fi; \
		if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$(APPLICATIONSDIR)"; fi; \
		if command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -f -t "$(DATADIR)/icons/hicolor"; fi; \
	fi
endif

# Android's native graph intentionally excludes the desktop editor and media backends.
ANDROID_SDK_ROOT ?= $(HOME)/Android/Sdk
ANDROID_NDK_VERSION ?= 28.2.13676358
ANDROID_NDK_HOME ?= $(ANDROID_SDK_ROOT)/ndk/$(ANDROID_NDK_VERSION)
ANDROID_TOOLCHAIN := $(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/linux-x86_64
ANDROID_ABI ?= arm64-v8a
ANDROID_TARGET = $(if $(filter arm64-v8a,$(ANDROID_ABI)),aarch64-linux-android,x86_64-linux-android)
ANDROID_CLANG = $(ANDROID_TARGET)33-clang
ANDROID_SERIAL ?=
ANDROID_ADB = adb -s "$(ANDROID_SERIAL)"
ANDROID_PROFILE ?= debug
ANDROID_DEV_SIGN ?= false
ANDROID_GRADLE ?= $(CURDIR)/android/gradlew
# rust-skia's unversioned target otherwise hides newer bionic declarations.
ANDROID_SKIA_CFLAGS = "--target=$(ANDROID_TARGET)33","$(if $(filter release,$(ANDROID_PROFILE)),-O3,-O0)","-I$(ANDROID_MEDIA_PREFIX)/include"
ifeq ($(ANDROID_ABI),arm64-v8a)
ANDROID_SKIA_CFLAGS += ,"-mno-outline-atomics"
endif
ANDROID_ENV = HOST_CC="$(CC)" HOST_CXX="$(CXX)" SKIA_GN_ARGS='ndk_api=33 skia_use_ndk_images=false skia_use_system_libjpeg_turbo=true extra_cflags=[] extra_cflags=[$(ANDROID_SKIA_CFLAGS)]' ANDROID_HOME="$(ANDROID_SDK_ROOT)" ANDROID_NDK_HOME="$(ANDROID_NDK_HOME)" ANDROID_NDK_ROOT="$(ANDROID_NDK_HOME)" ANDROID_NDK="$(ANDROID_NDK_HOME)" PATH="$(ANDROID_TOOLCHAIN)/bin:$(PATH)" CC="$(ANDROID_TOOLCHAIN)/bin/$(ANDROID_CLANG)" CXX="$(ANDROID_TOOLCHAIN)/bin/$(ANDROID_CLANG)++" AR="$(ANDROID_TOOLCHAIN)/bin/llvm-ar" RUSTFLAGS="-C linker=$(ANDROID_TOOLCHAIN)/bin/$(ANDROID_CLANG) -C link-arg=-Wl,-z,max-page-size=16384"
android-deps:
	@test "$(ANDROID_ABI)" = arm64-v8a -o "$(ANDROID_ABI)" = x86_64 || { echo "Unsupported Android ABI" >&2; exit 1; }
	@test "$(ANDROID_PROFILE)" = debug -o "$(ANDROID_PROFILE)" = release || { echo "Use ANDROID_PROFILE=debug or release" >&2; exit 1; }
	@test "$(ANDROID_PROFILE)" != release -o "$(ANDROID_ABI)" = arm64-v8a || { echo "Android releases require arm64-v8a" >&2; exit 1; }
	rustup target add $(ANDROID_TARGET)
	python3 packaging/android-native.py --ndk "$(ANDROID_NDK_HOME)" --cache "$(CARGO_TARGET_DIR)/android-native" --abi $(ANDROID_ABI) --profile $(ANDROID_PROFILE) --jobs $(or $(CARGO_BUILD_JOBS),4)
	ln -sfn "$(ANDROID_MEDIA_PREFIX)/lib/pkgconfig" "$(ANDROID_MEDIA_PREFIX)/../pkgconfig-$$(cat '$(ANDROID_MEDIA_PREFIX)/../build.sha256')"

android-native: android-deps
	$(ANDROID_ENV) $(ANDROID_MEDIA_ENV) PKG_CONFIG_ALL_STATIC=1 $(CARGO) build -p shrimply-editor-android --target $(ANDROID_TARGET) $(if $(filter release,$(ANDROID_PROFILE)),--release)
	mkdir -p android/app/src/main/jniLibs/$(ANDROID_ABI)
	cp $(CARGO_TARGET_DIR)/$(ANDROID_TARGET)/$(ANDROID_PROFILE)/libshrimply_android.so android/app/src/main/jniLibs/$(ANDROID_ABI)/
	cp "$(ANDROID_TOOLCHAIN)/sysroot/usr/lib/$(ANDROID_TARGET)/libc++_shared.so" android/app/src/main/jniLibs/$(ANDROID_ABI)/

android-build: android-native
	@test -n "$(ANDROID_GRADLE)" || { echo "Set ANDROID_GRADLE to Gradle 9.8.0's executable" >&2; exit 1; }
	ANDROID_HOME="$(ANDROID_SDK_ROOT)" "$(ANDROID_GRADLE)" -p android -PnativeAbi=$(ANDROID_ABI) -PdevSigning=$(ANDROID_DEV_SIGN) assemble$(if $(filter release,$(ANDROID_PROFILE)),Release,Debug)

android-install: ANDROID_DEV_SIGN = true
android-install: android-device-check android-build
	$(ANDROID_ADB) install -r android/app/build/outputs/apk/$(ANDROID_PROFILE)/app-$(ANDROID_PROFILE)$(if $(and $(filter release,$(ANDROID_PROFILE)),$(filter-out true,$(ANDROID_DEV_SIGN))),-unsigned).apk

android-check: android-deps android-dependency-check
	$(ANDROID_ENV) $(ANDROID_MEDIA_ENV) PKG_CONFIG_ALL_STATIC=1 $(CARGO) clippy -p shrimply-editor-android -p shrimply-timeline-paint-skia --target $(ANDROID_TARGET) $(if $(filter release,$(ANDROID_PROFILE)),--release) --no-deps -- -D warnings
	ANDROID_HOME="$(ANDROID_SDK_ROOT)" "$(ANDROID_GRADLE)" -p android -PnativeAbi=$(ANDROID_ABI) compile$(if $(filter release,$(ANDROID_PROFILE)),Release,Debug)Kotlin lint$(if $(filter release,$(ANDROID_PROFILE)),Release,Debug)

android-benchmark: android-device-check
	ANDROID_SERIAL="$(ANDROID_SERIAL)" python3 packaging/android-benchmark.py $(or $(STRESS),false)

android-dependency-check:
	mkdir -p $(CARGO_TARGET_DIR)
	@$(CARGO) tree -p shrimply-editor-android --target $(ANDROID_TARGET) --prefix none --edges normal > $(CARGO_TARGET_DIR)/android-dependencies.txt
	@if rg '^(gtk4[^ ]*|gdk4[^ ]*|shrimply-visual-cuda|shrimply-preview-runtime-cuda|shrimply-editor-gtk) ' $(CARGO_TARGET_DIR)/android-dependencies.txt; then echo "Desktop dependency leaked into Android" >&2; exit 1; fi

android-device-check:
	@test -n "$(ANDROID_SERIAL)" || { echo "Set ANDROID_SERIAL to the intended device (wireless IP:port or USB serial)" >&2; exit 1; }

ANDROID_MEDIA_PREFIX = $(abspath $(CARGO_TARGET_DIR)/android-native/$(ANDROID_ABI)/$(ANDROID_PROFILE)/install)
# ffmpeg-sys bundles static archives inside its rlib. A changed recipe must
# change its watched pkg-config environment, rather than only relink the app.
ANDROID_MEDIA_PKG_CONFIG = $(ANDROID_MEDIA_PREFIX)/../pkgconfig-$(shell cat "$(ANDROID_MEDIA_PREFIX)/../build.sha256" 2>/dev/null)
ANDROID_MEDIA_ENV = SYSTEM_DEPS_LINK=static PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_LIBDIR="$(ANDROID_MEDIA_PKG_CONFIG)" PKG_CONFIG_PATH="$(ANDROID_MEDIA_PKG_CONFIG)" BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$(ANDROID_TOOLCHAIN)/sysroot --target=$(ANDROID_TARGET)33"

.PHONY: build release dev dev-qt dev-mac qt-build qt-release install uninstall clean
.PHONY: native-deps linux-ffmpeg qt-native-deps cuda-target-check deps-fedora crash-report
.PHONY: appkit-ffmpeg appkit-icon appkit-release appkit-check appkit-components-showcase
.PHONY: windows-native-deps windows-build windows-dev windows-check windows-package
.PHONY: android-deps android-native android-build android-check android-install android-device-check android-benchmark android-dependency-check
.PHONY: check lint test fmt fmt-check docs docs-check dev-server server-python-check manim manim-python-check
.PHONY: gtk-components-showcase qt-components-showcase sanim-check project-server project-server-release project-server-check
