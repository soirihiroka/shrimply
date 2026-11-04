{ autoPatchelfHook, fetchurl, gcc, level-zero, lib, stdenv }:
stdenv.mkDerivation rec {
  pname = "shrimply-oidn";
  version = "2.5.1";
  src = fetchurl {
    url = "https://github.com/RenderKit/oidn/releases/download/v${version}/oidn-${version}.x86_64.linux.tar.gz";
    sha256 = "743c3e2aff8c220d5d70fe6cb970fb3d36f2702d2693c61d1d148e404cf37cd6";
  };
  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ gcc.cc.lib level-zero ];
  autoPatchelfIgnoreMissingDeps = [ "libcuda.so.1" ];
  dontBuild = true;
  installPhase = ''
    mkdir -p "$out/lib" "$out/share/licenses/oidn"
    cp -a lib/*.so* "$out/lib/"
    # Shrimply only uses the CUDA and SYCL GPU backends.
    rm -f "$out/lib/"*device_cpu* "$out/lib/"*device_hip* "$out/lib/"libtbb*
    cp -a doc/LICENSE* doc/third-party-programs*.txt "$out/share/licenses/oidn/"
  '';
  meta = {
    description = "Open Image Denoise GPU runtime for Shrimply";
    homepage = "https://www.openimagedenoise.org";
    license = lib.licenses.asl20;
    platforms = [ "x86_64-linux" ];
  };
}
