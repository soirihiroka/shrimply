#!/usr/bin/env python3
"""Cross-build Shrimply's existing native media/graphics dependencies with the Android NDK."""
import argparse
import fcntl
import hashlib
import os
import re
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

FFMPEG_VERSION = "9.0.2"
FFMPEG_SHA256 = "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
OPUS_VERSION = "1.6.1"
OPUS_SHA256 = "6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1"
RUBBERBAND_VERSION = "4.0.0"
RUBBERBAND_SHA256 = "af050313ee63bc18b35b2e064e5dce05b276aaf6d1aa2b8a82ced1fe2f8028e9"
PANGO_VERSION = "1.58.2"
PANGO_SHA256 = "342385b6ca3b7c73455d7c80a13b7dbe4489e00bc3bd4c5bd6ed4dce421e374a"
WRAPDB_REVISION = "cc81edad31940dce5d9595905529a5899a4319b4"
WRAPDB_SHA256 = "a1f3dbf2c7385be2f612c50b7fa949d3e1fce27752d742d1739b2081437cb1db"
OPENCV_VERSION = "4.13.0"
OPENCV_SHA256 = "1d40ca017ea51c533cf9fd5cbde5b5fe7ae248291ddf2af99d4c17cf8e13017d"
POPPLER_VERSION = "26.01.0"
POPPLER_SHA256 = "1cb944a4b88847f5fb6551683bc799db59f04990f5d8be07aba2acbf38601089"
LCMS_VERSION = "2.20"
LCMS_SHA256 = "18ff5504fa9cc8a2b1f3591180d90ead16c8a2ea78e530523b46eab70e1ca6b9"
ANDROID_API = 33
ABIS = {
    "arm64-v8a": ("aarch64", "aarch64-linux-android"),
    "x86_64": ("x86_64", "x86_64-linux-android"),
}


def source(cache, name, url, digest):
    archive = cache / url.rsplit("/", 1)[1]
    if not archive.exists():
        temporary = archive.with_suffix(archive.suffix + ".part")
        try:
            with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as output:
                shutil.copyfileobj(response, output)
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    with archive.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if actual != digest:
        raise RuntimeError(f"checksum mismatch for {archive.name}")
    directory = cache / name
    if not directory.exists():
        with tempfile.TemporaryDirectory(prefix="extract-", dir=cache) as staging:
            with tarfile.open(archive) as compressed:
                compressed.extractall(staging, filter="data")
            (Path(staging) / name).rename(directory)
    return directory


def build_rubberband(sources, root, prefix, compiler, toolchain, profile, ndk):
    identity = hashlib.sha256((RUBBERBAND_SHA256 + profile).encode() + (ndk / "source.properties").read_bytes()).hexdigest()
    stamp = root / "rubberband.sha256"
    library = prefix / "lib/librubberband.a"
    if stamp.exists() and stamp.read_text() == identity and library.is_file():
        return
    directory = source(sources, f"rubberband-{RUBBERBAND_VERSION}", f"https://breakfastquay.com/files/releases/rubberband-{RUBBERBAND_VERSION}.tar.bz2", RUBBERBAND_SHA256)
    output = root / "rubberband.o"
    # Upstream's supported single compilation unit includes its built-in FFT,
    # resampler and C API; the editor uses the same LiveShifter interface.
    subprocess.run([f"{compiler}++", "-std=c++11", "-fPIC", "-g", "-O0" if profile == "debug" else "-O3", "-c", str(directory / "single/RubberBandSingle.cpp"), "-o", str(output)], check=True)
    library.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(toolchain / "llvm-ar"), "rcs", str(library), str(output)], check=True)
    stamp.write_text(identity)


def patch_source(directory, patch):
    # The verified source cache is shared by ABIs; apply once under its own lock.
    with (directory.parent / f".{directory.name}-patch.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        command = ["patch", "--batch", "--silent", "-p1", "--input", str(patch)]
        if subprocess.run(command + ["--dry-run", "--forward"], cwd=directory, capture_output=True).returncode == 0:
            subprocess.run(command + ["--forward"], cwd=directory, check=True)
        else:
            # A changed upstream source or partial patch is a hard failure.
            subprocess.run(command + ["--dry-run", "--reverse"], cwd=directory, check=True)


def cmake_build(args, root, name, directory, options, required_cache=None):
    ndk, prefix, build = args.ndk.resolve(), root / "install", root / f"{name}-build"
    environment = os.environ.copy()
    environment.update(PKG_CONFIG_LIBDIR=str(prefix / "lib/pkgconfig"), PKG_CONFIG_PATH=str(prefix / "lib/pkgconfig"), PKG_CONFIG_ALLOW_CROSS="1")
    common = [
        f"-DCMAKE_TOOLCHAIN_FILE={ndk}/build/cmake/android.toolchain.cmake",
        f"-DANDROID_ABI={args.abi}", f"-DANDROID_PLATFORM=android-{ANDROID_API}",
        f"-DCMAKE_BUILD_TYPE={'Debug' if args.profile == 'debug' else 'Release'}",
        f"-DCMAKE_INSTALL_PREFIX={prefix}", "-DCMAKE_INSTALL_LIBDIR=lib",
        "-DCMAKE_POSITION_INDEPENDENT_CODE=ON", "-DBUILD_SHARED_LIBS=OFF",
        f"-DCMAKE_FIND_ROOT_PATH={prefix}", f"-DCMAKE_PREFIX_PATH={prefix}",
        f"-DCMAKE_EXE_LINKER_FLAGS=-L{prefix}/lib -Wl,-z,max-page-size=16384",
        "-DCMAKE_POLICY_VERSION_MINIMUM=3.5", "-DPKG_CONFIG_EXECUTABLE=/usr/bin/pkg-config", "-DPKG_CONFIG_ARGN=--static",
    ]
    subprocess.run(["cmake", "-S", str(directory), "-B", str(build), "-G", "Ninja", *common, *options], env=environment, check=True)
    if required_cache and required_cache not in (build / "CMakeCache.txt").read_text():
        raise RuntimeError(f"Android {name} configuration is missing {required_cache}")
    subprocess.run(["cmake", "--build", str(build), "--parallel", str(args.jobs)], env=environment, check=True)
    subprocess.run(["cmake", "--install", str(build)], env=environment, check=True)
    return build


def build_text(args, sources, root, compiler):
    prefix, toolchain = root / "install", compiler.parent
    arch, profile, ndk, jobs = ABIS[args.abi][0], args.profile, args.ndk.resolve(), args.jobs
    identity = hashlib.sha256(Path(__file__).read_bytes() + (ndk / "source.properties").read_bytes()).hexdigest()
    stamp = root / "text.sha256"
    if stamp.exists() and stamp.read_text() == identity and (prefix / "lib/libpangocairo-1.0.a").is_file():
        print(f"Android native text dependencies are current: {prefix}", flush=True)
        return
    pango = source(sources, f"pango-{PANGO_VERSION}", f"https://download.gnome.org/sources/pango/1.58/pango-{PANGO_VERSION}.tar.xz", PANGO_SHA256)
    wraps = source(sources, f"wrapdb-{WRAPDB_REVISION}", f"https://codeload.github.com/mesonbuild/wrapdb/tar.gz/{WRAPDB_REVISION}", WRAPDB_SHA256)
    directory = root / f"pango-{PANGO_VERSION}"
    shutil.copytree(pango, directory, dirs_exist_ok=True)
    # Pin fallback sources and Meson port files instead of following upstream's
    # development-branch wraps. Meson verifies every dependency archive hash.
    for wrap in (wraps / "subprojects").glob("*.wrap"):
        shutil.copy2(wrap, directory / "subprojects" / wrap.name)
    freetype_wrap = directory / "subprojects/freetype2.wrap"
    freetype_wrap.write_text(freetype_wrap.read_text().replace("https://download.savannah.gnu.org/", "https://download-mirror.savannah.gnu.org/"))
    shutil.copytree(wraps / "subprojects/packagefiles", directory / "subprojects/packagefiles", dirs_exist_ok=True)
    cross = root / "text-cross.ini"
    cross.write_text(f"""[binaries]
c = '{compiler}'
cpp = '{compiler}++'
ar = '{toolchain}/llvm-ar'
strip = '{toolchain}/llvm-strip'
pkg-config = '/usr/bin/pkg-config'
[host_machine]
system = 'android'
cpu_family = '{arch}'
cpu = '{arch}'
endian = 'little'
[properties]
needs_exe_wrapper = true
pkg_config_libdir = ['{prefix}/lib/pkgconfig']
[built-in options]
c_args = ['-fPIC']
cpp_args = ['-fPIC']
c_link_args = ['-Wl,-z,max-page-size=16384']
cpp_link_args = ['-Wl,-z,max-page-size=16384']
""")
    build = root / "text-build"
    options = [
        "-Ddefault_library=static", "-Dauto_features=disabled", "-Db_staticpic=true",
        "-Dbuild-testsuite=false", "-Dbuild-examples=false", "-Dintrospection=disabled",
        "-Dcairo=enabled", "-Dfreetype=enabled", "-Dfontconfig=enabled",
        "-Dglib:tests=false", "-Dglib:nls=disabled", "-Dglib:libmount=disabled",
        "-Dglib:selinux=disabled", "-Dglib:sysprof=disabled",
        "-Dcairo:freetype=enabled", "-Dcairo:fontconfig=enabled", "-Dcairo:glib=enabled", "-Dcairo:png=enabled",
        "-Dfreetype2:png=enabled", "-Dfreetype2:zlib=system",
        "-Dfontconfig:xml-backend=expat", "-Dfontconfig:cache-build=disabled",
        "-Dfontconfig:tests-external-fonts=disabled",
        "-Dharfbuzz:freetype=enabled", "-Dharfbuzz:glib=enabled", "-Dharfbuzz:gobject=enabled",
    ]
    subprocess.run(["meson", "setup", str(build), str(directory), "--reconfigure", "--cross-file", str(cross), f"--prefix={prefix}", "--libdir=lib", "--wrap-mode=default", f"--buildtype={'debug' if profile == 'debug' else 'release'}", *options], check=True)
    subprocess.run(["meson", "compile", "-C", str(build), "-j", str(jobs)], check=True)
    subprocess.run(["meson", "install", "-C", str(build), "--no-rebuild"], check=True)
    stamp.write_text(identity)


def build_opencv(args, sources, root):
    ndk, prefix = args.ndk.resolve(), root / "install"
    stamp = root / "opencv.sha256"
    compatibility_patch = Path(__file__).resolve().with_name("opencv-ffmpeg9.patch")
    identity = hashlib.sha256(Path(__file__).read_bytes() + compatibility_patch.read_bytes() + (ndk / "source.properties").read_bytes()).hexdigest()
    if stamp.exists() and stamp.read_text() == identity and (prefix / "lib/pkgconfig/opencv4.pc").is_file():
        print(f"Android OpenCV dependencies are current: {prefix}", flush=True)
        return
    directory = source(sources, f"opencv-{OPENCV_VERSION}", f"https://codeload.github.com/opencv/opencv/tar.gz/refs/tags/{OPENCV_VERSION}", OPENCV_SHA256)
    patch_source(directory, compatibility_patch)
    options = [
        "-DBUILD_LIST=core,imgproc,video,videoio,calib3d", "-DBUILD_TESTS=OFF",
        "-DBUILD_PERF_TESTS=OFF", "-DBUILD_EXAMPLES=OFF", "-DBUILD_ANDROID_PROJECTS=OFF",
        "-DBUILD_ANDROID_EXAMPLES=OFF", "-DBUILD_JAVA=OFF", "-DBUILD_opencv_apps=OFF",
        "-DWITH_IPP=OFF", "-DWITH_TBB=OFF", "-DWITH_FFMPEG=ON", "-DWITH_GSTREAMER=OFF",
        "-DOPENCV_GENERATE_PKGCONFIG=ON", "-DOPENCV_LIB_INSTALL_PATH=lib",
        "-DOPENCV_LIB_ARCHIVE_INSTALL_PATH=lib", "-DOPENCV_3P_LIB_INSTALL_PATH=lib",
        "-DOPENCV_INCLUDE_INSTALL_PATH=include/opencv4",
    ]
    build = cmake_build(args, root, "opencv", directory, options, "__VALID_FFMPEG:INTERNAL=TRUE")
    # OpenCV generates this metadata but omits its installation on Android.
    shutil.copy2(build / "unix-install/opencv4.pc", prefix / "lib/pkgconfig/opencv4.pc")
    # Reuse these exact native image decoders in Poppler as well. OpenCV ships
    # their archives but leaves out the public headers and package metadata.
    jpeg = directory / "3rdparty/libjpeg-turbo"
    openjpeg = directory / "3rdparty/openjpeg/openjp2"
    for name, headers in {
        "jpeg": [jpeg / "src" / header for header in ("jpeglib.h", "jmorecfg.h", "jerror.h")] + [build / "3rdparty/libjpeg-turbo/jconfig.h"],
        "openjpeg": [openjpeg / header for header in ("openjpeg.h", "opj_stdint.h")] + [build / "3rdparty/openjpeg/openjp2/opj_config.h"],
    }.items():
        destination = prefix / "include" / name
        destination.mkdir(parents=True, exist_ok=True)
        for header in headers:
            shutil.copy2(header, destination / header.name)
    jpeg_version = re.search(r"set\(VERSION ([0-9.]+)\)", (jpeg / "CMakeLists.txt").read_text()).group(1)
    openjpeg_version = ".".join(re.findall(r"#define OPJ_VERSION_\w+ (\d+)", (build / "3rdparty/openjpeg/openjp2/opj_config.h").read_text()))
    for name, version, library, includes in (("libjpeg", jpeg_version, "libjpeg-turbo", "jpeg"), ("libopenjp2", openjpeg_version, "libopenjp2", "openjpeg")):
        (prefix / "lib/pkgconfig" / f"{name}.pc").write_text(f"prefix={prefix}\nlibdir=${{prefix}}/lib\nincludedir=${{prefix}}/include/{includes}\nName: {name}\nDescription: Shared native {name} decoder\nVersion: {version}\nLibs: -L${{libdir}} -l{library}\nLibs.private: -lm\nCflags: -I${{includedir}}\n")
    config = prefix / "lib/cmake/openjpeg"
    config.mkdir(parents=True, exist_ok=True)
    (config / "OpenJPEGConfig.cmake").write_text(f"set(OPENJPEG_MAJOR_VERSION {openjpeg_version.split('.')[0]})\nset(OPENJPEG_INCLUDE_DIRS \"{prefix}/include/openjpeg\")\nset(OPENJPEG_LIBRARIES openjp2)\nif(NOT TARGET openjp2)\nadd_library(openjp2 STATIC IMPORTED)\nset_target_properties(openjp2 PROPERTIES IMPORTED_LOCATION \"{prefix}/lib/liblibopenjp2.a\" INTERFACE_INCLUDE_DIRECTORIES \"{prefix}/include/openjpeg\")\nendif()\n")
    stamp.write_text(identity)


def build_pdf(args, sources, root):
    prefix, ndk = root / "install", args.ndk.resolve()
    stamp = root / "pdf.sha256"
    identity = hashlib.sha256(Path(__file__).read_bytes() + (ndk / "source.properties").read_bytes()).hexdigest()
    if stamp.exists() and stamp.read_text() == identity and (prefix / "lib/libpoppler-glib.a").is_file():
        print(f"Android PDF dependencies are current: {prefix}", flush=True)
        return
    lcms = source(sources, f"lcms2-{LCMS_VERSION}", f"https://github.com/mm2/Little-CMS/releases/download/lcms{LCMS_VERSION}/lcms2-{LCMS_VERSION}.tar.gz", LCMS_SHA256)
    lcms_build = root / "lcms-build"
    subprocess.run(["meson", "setup", str(lcms_build), str(lcms), "--reconfigure", "--cross-file", str(root / "text-cross.ini"), f"--prefix={prefix}", "--libdir=lib", f"--buildtype={'debug' if args.profile == 'debug' else 'release'}", "-Ddefault_library=static", "-Db_staticpic=true", "-Dauto_features=disabled", "-Dtests=disabled", "-Dutils=false"], check=True)
    subprocess.run(["meson", "compile", "-C", str(lcms_build), "-j", str(args.jobs)], check=True)
    subprocess.run(["meson", "install", "-C", str(lcms_build), "--no-rebuild"], check=True)
    directory = source(sources, f"poppler-{POPPLER_VERSION}", f"https://poppler.freedesktop.org/poppler-{POPPLER_VERSION}.tar.xz", POPPLER_SHA256)
    cmake_build(args, root, "poppler", directory, [
        "-DBUILD_MANUAL_TESTS=OFF", "-DENABLE_UTILS=OFF", "-DENABLE_CPP=OFF",
        "-DENABLE_GLIB=ON", "-DENABLE_GOBJECT_INTROSPECTION=OFF", "-DENABLE_GTK_DOC=OFF",
        "-DENABLE_QT5=OFF", "-DENABLE_QT6=OFF", "-DBUILD_GTK_TESTS=OFF",
        "-DBUILD_QT5_TESTS=OFF", "-DBUILD_QT6_TESTS=OFF", "-DBUILD_CPP_TESTS=OFF",
        "-DENABLE_BOOST=OFF", "-DENABLE_LIBCURL=OFF", "-DENABLE_LIBTIFF=OFF",
        "-DENABLE_NSS3=OFF", "-DENABLE_GPGME=OFF", "-DCMAKE_DISABLE_FIND_PACKAGE_GTK=TRUE",
        "-DENABLE_LIBOPENJPEG=openjpeg2", "-DENABLE_LIBJPEG=ON", "-DENABLE_LCMS=ON",
        f"-DOpenJPEG_DIR={prefix}/lib/cmake/openjpeg",
        f"-DJPEG_LIBRARY={prefix}/lib/liblibjpeg-turbo.a", f"-DJPEG_INCLUDE_DIR={prefix}/include/jpeg",
    ])
    if not (prefix / "lib/libpoppler-glib.a").is_file():
        raise RuntimeError("Android Poppler did not build the shared PDF crate's GLib/Cairo backend")
    stamp.write_text(identity)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ndk", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--abi", choices=ABIS, default="arm64-v8a")
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--text", action="store_true", help="Build the existing Pango/Cairo text dependencies")
    parser.add_argument("--opencv", action="store_true", help="Build the existing OpenCV analysis dependencies")
    parser.add_argument("--pdf", action="store_true", help="Build the existing Poppler PDF dependencies")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    ndk = args.ndk.resolve()
    cache = args.cache.resolve()
    root = cache / args.abi / args.profile
    root.mkdir(parents=True, exist_ok=True)
    # These are task-owned dependency outputs; never clean other target artifacts.
    with (root / ".lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        toolchain = ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin"
        arch, triple = ABIS[args.abi]
        compiler = toolchain / f"{triple}{ANDROID_API}-clang"
        if not compiler.is_file():
            raise RuntimeError(f"Android NDK compiler not found: {compiler}")
        prefix = root / "install"
        package_directory = prefix / "lib/pkgconfig"
        package_directory.mkdir(parents=True, exist_ok=True)
        zlib_header = toolchain.parent / "sysroot/usr/include/zlib.h"
        zlib_version = re.search(r'#define ZLIB_VERSION "([^"]+)"', zlib_header.read_text()).group(1)
        (package_directory / "zlib.pc").write_text(f"Name: zlib\nDescription: Android NDK system zlib\nVersion: {zlib_version}\nLibs: -lz\n")
        stamp = root / "build.sha256"
        hardware_patch = Path(__file__).resolve().with_name("ffmpeg-android-hardware.patch")
        encoder_patch = Path(__file__).resolve().with_name("ffmpeg-android-encoder.patch")
        identity = hashlib.sha256(Path(__file__).read_bytes() + hardware_patch.read_bytes() + encoder_patch.read_bytes() + (ndk / "source.properties").read_bytes()).hexdigest()
        sources = cache / "sources"
        sources.mkdir(exist_ok=True)
        if args.text:
            build_text(args, sources, root, compiler)
            return
        if args.opencv:
            build_opencv(args, sources, root)
            return
        if args.pdf:
            build_pdf(args, sources, root)
            return
        build_rubberband(sources, root, prefix, compiler, toolchain, args.profile, ndk)
        if stamp.exists() and stamp.read_text() == identity and (prefix / "lib/libavcodec.a").is_file():
            print(f"Android media dependencies are current: {prefix}", flush=True)
            return
        sources = cache / "sources"
        sources.mkdir(exist_ok=True)
        opus = source(sources, f"opus-{OPUS_VERSION}", f"https://downloads.xiph.org/releases/opus/opus-{OPUS_VERSION}.tar.gz", OPUS_SHA256)
        ffmpeg = source(sources, f"ffmpeg-{FFMPEG_VERSION}", f"https://ffmpeg.org/releases/ffmpeg-{FFMPEG_VERSION}.tar.xz", FFMPEG_SHA256)
        patch_source(ffmpeg, hardware_patch)
        patch_source(ffmpeg, encoder_patch)
        cmake_build(args, root, "opus", opus, ["-DOPUS_BUILD_TESTING=OFF", "-DOPUS_BUILD_PROGRAMS=OFF"])
        ffmpeg_build = root / "ffmpeg"
        ffmpeg_build.mkdir(exist_ok=True)
        environment = os.environ.copy()
        environment.update({
            "PATH": f"{toolchain}{os.pathsep}{environment['PATH']}",
            "PKG_CONFIG_PATH": str(prefix / "lib/pkgconfig"),
            "PKG_CONFIG_LIBDIR": str(prefix / "lib/pkgconfig"),
            "PKG_CONFIG_SYSROOT_DIR": "",
            "PKG_CONFIG_ALLOW_CROSS": "1",
        })
        flags = [
            str(ffmpeg / "configure"), f"--prefix={prefix}",
            "--target-os=android", f"--arch={arch}", "--enable-cross-compile",
            f"--cc={compiler}", f"--cxx={compiler}++", f"--ar={toolchain}/llvm-ar",
            f"--nm={toolchain}/llvm-nm", f"--ranlib={toolchain}/llvm-ranlib",
            f"--strip={toolchain}/llvm-strip", "--enable-pic", "--enable-static", "--disable-shared",
            "--disable-autodetect", "--disable-programs", "--disable-doc", "--disable-stripping",
            "--enable-mediacodec", "--enable-jni", "--enable-libopus", "--enable-debug=2",
            "--pkg-config-flags=--static",
            "--extra-ldflags=-Wl,-z,max-page-size=16384",
        ]
        if args.profile == "debug":
            flags += ["--disable-optimizations", "--extra-cflags=-O0 -g"]
        subprocess.run(flags, cwd=ffmpeg_build, env=environment, check=True)
        subprocess.run(["make", f"-j{args.jobs}"], cwd=ffmpeg_build, env=environment, check=True)
        subprocess.run(["make", "install"], cwd=ffmpeg_build, env=environment, check=True)
        stamp.write_text(identity)
        print(f"Android media dependencies installed: {prefix}", flush=True)


if __name__ == "__main__":
    main()
