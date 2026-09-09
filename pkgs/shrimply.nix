{
  alsa-lib,
  boost,
  clang,
  cmake,
  cudaPackages,
  fetchurl,
  ffmpeg,
  freetype,
  gcc,
  gnumake,
  gobject-introspection,
  gtk4,
  gtksourceview5,
  lib,
  libadwaita,
  libglvnd,
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
  rustPlatform,
  shrimply-slang,
  uv,
  vte-gtk4,
  vtracerSrc,
  wrapGAppsHook4,
}:
let
  cudaStubs = lib.getOutput "stubs" cudaPackages.cuda_cudart;
  cudaToolkit = cudaPackages.cudatoolkit;
  skiaBinaries = fetchurl {
    url = "https://github.com/rust-skia/skia-binaries/releases/download/0.99.0/skia-binaries-a25a0fdb7d90429aa2d1-x86_64-unknown-linux-gnu-egl-gl-jpegd-jpege-pdf-skottie-svg-textlayout-vulkan-wayland-webpd-webpe-x11.tar.gz";
    hash = "sha256-u+Oec2kkFbGy5dVZGcaUisun0WryJ3xmTK6cHHe2OzA=";
  };
in
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
      'Path::new("'"$out"'/share/shrimply/crates/media/visual/manim/manim-bridge/python")'
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
    ffmpeg
    freetype
    gtk4
    gtksourceview5
    libadwaita
    libglvnd
    opencv
    openssl
    pipewire
    poppler_gi
    rubberband
    shrimply-slang
    vte-gtk4
  ];

  CARGO = "cargo";
  RUSTC = "rustc";
  CUDA_HOME = cudaToolkit;
  CUDA_HOST_CXX = "g++";
  CUDA_TOOLKIT_PATH = cudaToolkit;
  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
  NIX_LDFLAGS = "-L${cudaStubs}/lib/stubs -L${shrimply-slang}/lib";
  PKG_CONFIG = "pkg-config";
  SKIA_BINARIES_URL = "file://${skiaBinaries}";
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
