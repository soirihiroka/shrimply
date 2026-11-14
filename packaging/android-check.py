"""Validate every packaged ELF and the APK's 16 KiB ZIP alignment."""
import pathlib
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
        print(f"{name}: ELF LOAD alignment >= 16 KiB")
zipalign = sdk / "build-tools/37.0.0/zipalign"
subprocess.run([zipalign, "-c", "-P", "16", "-v", "4", apk], check=True, stdout=subprocess.DEVNULL)
print(f"{apk}: APK alignment supports 16 KiB pages")
