"""Build vcpkg's FFmpeg and its host utility from the editor's FFmpeg submodule."""
import json
import os
from pathlib import Path
import shutil
import sys

from native_sources import FFMPEG, ffmpeg_revision


def main():
    revision = ffmpeg_revision()
    version = (FFMPEG / "RELEASE").read_text().strip()
    for name in ("ffmpeg", "ffmpeg-bin2c"):
        overlay = Path(sys.argv[2]).resolve() / name
        shutil.copytree(Path(sys.argv[1]) / "ports" / name, overlay, dirs_exist_ok=True)
        manifest = overlay / "vcpkg.json"
        metadata = json.loads(manifest.read_text())
        metadata["version"] = version
        metadata.pop("port-version", None)
        manifest.write_text(json.dumps(metadata, indent=2) + "\n")

        # Keep the upstream configuration, features and package layout. Only replace
        # source acquisition and the patches needed by the Windows editor build.
        port = overlay / "portfile.cmake"
        before, start, rest = port.read_text().partition("vcpkg_from_github(\n")
        source, end, build = rest.partition("\n)\n")
        if not start or not end or "REPO ffmpeg/ffmpeg" not in source:
            raise ValueError(f"Unexpected source declaration in the pinned vcpkg {name} port")
        local_repo = Path(os.path.relpath(FFMPEG, overlay)).as_posix()
        declaration = f'''vcpkg_from_git(
    OUT_SOURCE_PATH SOURCE_PATH
    URL "${{CMAKE_CURRENT_LIST_DIR}}/{local_repo}"
    REF {revision}
'''
        if name == "ffmpeg":
            # 9.1 removed the OpenGL output device; our port does not request it.
            build = build.replace(" --disable-opengl", "")
            patch = Path(__file__).with_name("windows") / "vcpkg/prebuilt-bin2c.patch"
            shutil.copyfile(patch, overlay / patch.name)
            declaration += '''    PATCHES
        0003-fix-windowsinclude.patch
        0005-fix-nasm.patch
        0013-define-WINVER.patch
        prebuilt-bin2c.patch
        0046-fix-msvc-detection.patch
        0050-fix-test-ld-absolute-lib-paths.patch
        0051-fix-msvc-undef-flags.patch
'''
        version_file = 'configure_file("${SOURCE_PATH}/RELEASE" "${SOURCE_PATH}/FF_VERSION" COPYONLY)'
        port.write_text(before + declaration + ")\n" + version_file + "\n" + build)


if __name__ == "__main__":
    main()
