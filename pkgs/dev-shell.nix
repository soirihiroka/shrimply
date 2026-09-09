{
  alsa-lib,
  boost,
  clang,
  cmake,
  cudaPackages,
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
  mkShell,
  ninja,
  opencv,
  openssl,
  pkg-config,
  pipewire,
  poppler_gi,
  python3,
  qt6,
  rubberband,
  rustToolchain,
  shrimply-slang,
  uv,
  vte-gtk4,
}:
let
  cudaStubs = lib.getOutput "stubs" cudaPackages.cuda_cudart;
  cudaToolkit = cudaPackages.cudatoolkit;
  qtEnv = qt6.env "shrimply-qt" [
    qt6.qtbase
    qt6.qtdeclarative
    qt6.qtwayland
  ];
in
mkShell {
  packages = [
    rustToolchain
    cudaToolkit
    alsa-lib
    boost
    clang
    cmake
    ffmpeg
    freetype
    gcc
    gnumake
    gobject-introspection
    gtk4
    gtksourceview5
    libadwaita
    libglvnd
    lld
    ninja
    opencv
    openssl
    pkg-config
    pipewire
    poppler_gi
    python3
    qtEnv
    rubberband
    shrimply-slang
    uv
    vte-gtk4
  ];

  CARGO = "cargo";
  RUSTC = "rustc";
  CUDA_HOME = cudaToolkit;
  CUDA_HOST_CXX = "g++";
  CUDA_TOOLKIT_PATH = cudaToolkit;
  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
  NIX_CC_USE_RESPONSE_FILE = "1";
  NIX_LDFLAGS = "-L${cudaStubs}/lib/stubs -L${shrimply-slang}/lib";
  PKG_CONFIG = "pkg-config";
  QT_QMAKE = "${qtEnv}/bin/qmake";
  SLANG_INCLUDE_DIR = "${shrimply-slang}/include";
  SLANG_LIBRARY_DIR = "${shrimply-slang}/lib";

  shellHook = ''
    export NIX_CFLAGS_COMPILE=
    export NIX_LDFLAGS="-L${cudaStubs}/lib/stubs -L${shrimply-slang}/lib"
    export LD_LIBRARY_PATH="${
      lib.makeLibraryPath [
        gcc.cc.lib
        opencv
        shrimply-slang
        qtEnv
      ]
    }:/run/opengl-driver/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  '';
}
