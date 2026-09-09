{
  autoPatchelfHook,
  fetchurl,
  lib,
  stdenv,
}:
# Mirrors the prebuilt release pinned by crates/gpu/slang/slang-build/build.rs,
# so Nix builds link the same Slang the upstream build scripts download.
stdenv.mkDerivation (finalAttrs: {
  pname = "slang";
  version = "2026.17";

  src = fetchurl {
    url = "https://github.com/shader-slang/slang/releases/download/v${finalAttrs.version}/slang-${finalAttrs.version}-linux-x86_64-glibc-2.28.tar.gz";
    sha256 = "a5a48530e7218d79e10b633c216ef04cbe778450b8c0a7579125e630c088ca75";
  };

  sourceRoot = ".";

  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ stdenv.cc.cc.lib ];

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    # Only the library and headers are consumed; the CLI tools are unused.
    cp -r include lib "$out/"
    # libgfx is Slang's unused graphics layer and only adds an X11 dependency.
    rm "$out"/lib/libgfx.so*
    install -Dm644 LICENSE "$out/share/licenses/slang/LICENSE"
    runHook postInstall
  '';

  meta = {
    description = "Slang shading language compiler";
    homepage = "https://github.com/shader-slang/slang";
    license = lib.licenses.asl20;
    platforms = [ "x86_64-linux" ];
  };
})
