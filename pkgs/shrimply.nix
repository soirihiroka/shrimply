{
  alsa-lib,
  boost,
  clang,
  cmake,
  cudaPackages,
  expat,
  fetchFromGitHub,
  ffmpeg,
  fontconfig,
  freetype,
  gcc,
  gn,
  gnumake,
  gobject-introspection,
  gtk4,
  gtksourceview5,
  harfbuzzFull,
  icu,
  lib,
  libadwaita,
  libglvnd,
  libjpeg,
  libpng,
  libwebp,
  libx11,
  lld,
  llvmPackages,
  manimSrc,
  makeShellWrapper,
  ninja,
  opencv,
  openssl,
  optixSrc,
  pipewire,
  pkg-config,
  poppler_gi,
  python3,
  rhubarbSrc,
  rubberband,
  runCommand,
  rustPlatform,
  shrimply-slang,
  uv,
  vulkan-headers,
  vulkan-memory-allocator,
  vte-gtk4,
  vtracerSrc,
  wrapGAppsHook4,
  zlib,
}:
let
  cudaStubs = lib.getOutput "stubs" cudaPackages.cuda_cudart;
  cudaToolkit = cudaPackages.cudatoolkit;
  skiaVersion = (
    lib.findFirst (package: package.name == "skia-bindings")
      (throw "Cargo.lock does not contain skia-bindings")
      (builtins.fromTOML (builtins.readFile ../Cargo.lock)).package
  ).version;
  skiaSrc = fetchFromGitHub {
    owner = "rust-skia";
    repo = "skia";
    tag = "m153-0.101.2";
    hash = "sha256-xKCkvQoTAMlLvKdVzzSxTST6kER4kRUIDjvTRZ9z72o=";
  };
  spirvCrossSrc = fetchFromGitHub {
    owner = "KhronosGroup";
    repo = "SPIRV-Cross";
    rev = "b8fcf307f1f347089e3c46eb4451d27f32ebc8d3";
    hash = "sha256-H43M9DXfEuyKuvo6rjb5k0KEbYOSFodbPJh8ZKY4PQg=";
  };
  wuffsSrc = fetchFromGitHub {
    owner = "google";
    repo = "wuffs-mirror-release-c";
    rev = "e3f919ccfe3ef542cfc983a82146070258fb57f8";
    hash = "sha256-373d2F/STcgCHEq+PO+SCHrKVOo6uO1rqqwRN5eeBCw=";
  };
  skiaSource = runCommand "shrimply-skia-${skiaVersion}-source" { } ''
    cp -r ${skiaSrc} "$out"
    chmod -R u+w "$out"
    mkdir -p "$out/third_party/externals"
    ln -s ${spirvCrossSrc} "$out/third_party/externals/spirv-cross"
    ln -s ${vulkan-memory-allocator.src} "$out/third_party/externals/vulkanmemoryallocator"
    ln -s ${wuffsSrc} "$out/third_party/externals/wuffs"
    substituteInPlace "$out/third_party/harfbuzz/BUILD.gn" \
      --replace-fail /usr/include/harfbuzz ${lib.getDev harfbuzzFull}/include/harfbuzz
  '';
in
assert lib.assertMsg (skiaVersion == "0.153.3")
  "Update the pinned Nix Skia sources to match skia-bindings ${skiaVersion}";
rustPlatform.buildRustPackage {
  pname = "shrimply";
  version = "0.1.0";

  src = null;
  cargoLock.lockFile = ../Cargo.lock;

  postUnpack = ''
    mkdir -p "$sourceRoot/external"
    for dependency in manim optix-dev vtracer; do
      rm -rf "$sourceRoot/external/$dependency"
    done
    cp -r ${manimSrc} "$sourceRoot/external/manim"
    cp -r ${optixSrc} "$sourceRoot/external/optix-dev"
    cp -r ${vtracerSrc} "$sourceRoot/external/vtracer"
    chmod -R u+w "$sourceRoot/external"
  '';

  postPatch = ''
    substituteInPlace crates/media/visual/manim/manim-bridge/src/lib.rs \
      --replace-fail 'Path::new(env!("CARGO_MANIFEST_DIR")).join("python")' \
      'PathBuf::from("'"$out"'/share/shrimply/crates/media/visual/manim/manim-bridge/python")'
  '';

  nativeBuildInputs = [
    clang
    cmake
    gcc
    gnumake
    gobject-introspection
    lld
    ninja
    pkg-config
    python3
    (wrapGAppsHook4.override { makeWrapper = makeShellWrapper; })
  ];

  buildInputs = [
    cudaToolkit
    alsa-lib
    boost
    expat
    ffmpeg
    fontconfig
    freetype
    gtk4
    gtksourceview5
    harfbuzzFull
    icu
    libadwaita
    libglvnd
    libjpeg
    libpng
    libwebp
    libx11
    opencv
    openssl
    pipewire
    poppler_gi
    rubberband
    shrimply-slang
    vulkan-headers
    vte-gtk4
    zlib
  ];

  CARGO = "cargo";
  RUSTC = "rustc";
  CUDA_HOME = cudaToolkit;
  CUDA_HOST_CXX = "g++";
  CUDA_TOOLKIT_PATH = cudaToolkit;
  CLANGCC = "${clang}/bin/clang";
  CLANGCXX = "${clang}/bin/clang++";
  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
  NIX_LDFLAGS = "-L${cudaStubs}/lib/stubs -L${shrimply-slang}/lib -lharfbuzz-subset -lwebpdemux -lwebpmux";
  PKG_CONFIG = "pkg-config";
  SKIA_SOURCE_DIR = skiaSource;
  SKIA_USE_SYSTEM_LIBRARIES = "1";
  SKIA_GN_COMMAND = "${gn}/bin/gn";
  SKIA_GN_ARGS = ''skia_system_freetype2_include_path="${lib.getDev freetype}/include/freetype2"'';
  SLANG_INCLUDE_DIR = "${shrimply-slang}/include";
  SLANG_LIBRARY_DIR = "${shrimply-slang}/lib";

  buildPhase = ''
    runHook preBuild
    export OPTIX_ROOT="$PWD/external/optix-dev"
    export SHRIMPLY_POCKETSPHINX_CACHE="$PWD/.pocketsphinx-cache"
    mkdir "$SHRIMPLY_POCKETSPHINX_CACHE"
    cp ${rhubarbSrc}/rhubarb/lib/cmusphinx-en-us-5.2/{mdef,means,variances,mixture_weights,transition_matrices,feature_transform} \
      "$SHRIMPLY_POCKETSPHINX_CACHE/"
    cp ${rhubarbSrc}/rhubarb/lib/pocketsphinx-rev13216/model/en-us/en-us-phone.lm.bin \
      "$SHRIMPLY_POCKETSPHINX_CACHE/phone_lm"
    make release
    runHook postBuild
  '';

  # The application tests require CUDA hardware, display services, and test fixtures.
  doCheck = false;

  installPhase = ''
    runHook preInstall

    install -Dm755 target/release/shrimply "$out/bin/shrimply"
    install -Dm755 target/release/shrimply-editor "$out/bin/shrimply-editor"
    install -Dm755 target/release/shrimply-mcp "$out/bin/shrimply-mcp"
    install -Dm644 target/release/res/lip-sync/pocketsphinx-ci.model \
      "$out/share/shrimply/lip-sync/pocketsphinx-ci.model"
    install -Dm644 vendor/pocketsphinx/LICENSE \
      "$out/share/licenses/shrimply/PocketSphinx-code.txt"
    install -Dm644 vendor/pocketsphinx/MODEL-LICENSE \
      "$out/share/licenses/shrimply/PocketSphinx-model.txt"
    install -Dm644 vendor/rhubarb-lip-sync/LICENSE \
      "$out/share/licenses/shrimply/Rhubarb-Lip-Sync.txt"

    # components-gtk resolves bundled icons relative to the binary.
    install -d "$out/share/shrimply/icons"
    cp -a assets/icons/. "$out/share/shrimply/icons/"
    install -Dm644 assets/icons/dev.shrimply.Shrimply.svg \
      "$out/share/icons/hicolor/scalable/apps/dev.shrimply.Shrimply.svg"
    install -Dm644 assets/icons/dev.shrimply.Shrimply-project.svg \
      "$out/share/icons/hicolor/scalable/mimetypes/dev.shrimply.Shrimply-project.svg"
    install -Dm644 assets/dev.shrimply.Shrimply-mime.xml \
      "$out/share/mime/packages/dev.shrimply.Shrimply-mime.xml"
    sed -e "s|^Exec=.*|Exec=$out/bin/shrimply %f|" \
      -e "s|^TryExec=.*|TryExec=$out/bin/shrimply|" \
      assets/dev.shrimply.Shrimply.desktop \
      > dev.shrimply.Shrimply.desktop
    install -Dm644 dev.shrimply.Shrimply.desktop \
      "$out/share/applications/dev.shrimply.Shrimply.desktop"

    # The Manim worker resolves its uv project relative to this layout.
    install -d "$out/share/shrimply/crates/media/visual/manim/manim-bridge" \
      "$out/share/shrimply/external"
    cp -r crates/media/visual/manim/manim-bridge/python \
      "$out/share/shrimply/crates/media/visual/manim/manim-bridge/python"
    cp -r external/manim "$out/share/shrimply/external/manim"

    runHook postInstall
  '';

  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : ${
        lib.makeBinPath [
          ffmpeg
          uv
        ]
      }
      --prefix LD_LIBRARY_PATH : /run/opengl-driver/lib
      --run 'export UV_PROJECT_ENVIRONMENT="''${UV_PROJECT_ENVIRONMENT:-''${XDG_CACHE_HOME:-$HOME/.cache}/shrimply/manim}"'
    )
  '';

  meta = {
    description = "Free and open-source video editor";
    homepage = "https://github.com/soirihiroka/shrimply";
    license = lib.licenses.gpl3Plus;
    mainProgram = "shrimply";
    platforms = [ "x86_64-linux" ];
  };
}
