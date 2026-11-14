"""Validate every packaged ELF and the APK's 16 KiB ZIP alignment."""
import pathlib
import re
import subprocess
import sys
import tempfile
import zipfile

apk, readelf, sdk = map(pathlib.Path, sys.argv[1:])
with zipfile.ZipFile(apk) as archive, tempfile.TemporaryDirectory() as directory:
    libraries = [name for name in archive.namelist() if name.endswith(".so")]
    if not libraries:
        raise SystemExit("APK contains no native libraries")
    for index, name in enumerate(libraries):
        library = pathlib.Path(directory) / f"{index}.so"
        library.write_bytes(archive.read(name))
        output = subprocess.check_output([readelf, "-lW", library], text=True)
        alignments = [int(line.split()[-1], 16) for line in output.splitlines() if line.strip().startswith("LOAD ")]
        if not alignments or any(alignment < 16384 for alignment in alignments):
            raise SystemExit(f"{name}: ELF LOAD alignment does not support 16 KiB pages")
        abi = pathlib.PurePosixPath(name).parent.name
        triple = {"arm64-v8a": "aarch64-linux-android", "x86_64": "x86_64-linux-android"}[abi]
        platform = readelf.parent.parent / "sysroot/usr/lib" / triple / "33"
        if not platform.is_dir():
            raise SystemExit(f"Missing Android API 33 library stubs: {platform}")
        dynamic = subprocess.check_output([readelf, "-dW", library], text=True)
        for needed in re.findall(r"\(NEEDED\).*\[([^]]+)\]", dynamic):
            packaged = str(pathlib.PurePosixPath(name).parent / needed)
            if packaged not in libraries and not (platform / needed).is_file():
                raise SystemExit(f"{name}: required library {needed} is neither packaged nor supplied by Android")
        print(f"{name}: ELF LOAD alignment >= 16 KiB")
zipalign = sdk / "build-tools/37.0.0/zipalign"
subprocess.run([zipalign, "-c", "-P", "16", "-v", "4", apk], check=True, stdout=subprocess.DEVNULL)
print(f"{apk}: APK alignment supports 16 KiB pages")
