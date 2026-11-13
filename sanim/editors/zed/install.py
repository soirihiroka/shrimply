"""Install a complete prebuilt extension; let Zed maintain its own index."""
import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--check-wasm", action="store_true")
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    manifest = tomllib.loads((source / "extension.toml").read_text())
    language = tomllib.loads((source / "languages/sanim/config.toml").read_text())
    if "grammars" in manifest or "grammar" in language:
        raise SystemExit("Sanim uses its language server's semantic tokens, without a second grammar")
    if manifest["language_servers"]["sanim-lsp"]["languages"] != [language["name"]]:
        raise SystemExit("language server and language configuration names must match")
    if language["path_suffixes"] != ["sanim"]:
        raise SystemExit("Sanim must recognize .sanim files")
    if args.check:
        print("Sanim Zed metadata: valid; highlighting comes from sanim-lsp")
        return
    wasm = (source / "extension.wasm").read_bytes()
    if not wasm.startswith(b"\0asm") or b"zed:api-version" not in wasm:
        raise SystemExit("extension.wasm is missing a WebAssembly module or Zed API version")
    if args.check_wasm:
        print(f"Sanim Zed WebAssembly: {len(wasm)} bytes; Zed API version present")
        return
    if sys.platform == "darwin":
        data = Path.home() / "Library/Application Support/Zed"
    elif sys.platform == "win32":
        data = Path(os.environ["LOCALAPPDATA"]) / "Zed"
    else:
        data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "zed"
    installed = args.directory or data / "extensions/installed"
    staging = installed.parent / "staging"
    installed.mkdir(parents=True, exist_ok=True)
    staging.mkdir(parents=True, exist_ok=True)
    destination = installed / manifest["id"]
    if destination.exists():
        previous = tomllib.loads((destination / "extension.toml").read_text())
        if previous["id"] != manifest["id"]:
            raise SystemExit(f"refusing to replace unrelated extension: {destination}")
    with tempfile.TemporaryDirectory(prefix="sanim-local-", dir=staging) as temporary:
        temporary = Path(temporary)
        ready = temporary / manifest["id"]
        ready.mkdir()
        shutil.copy2(source / "extension.toml", ready)
        shutil.copy2(source / "extension.wasm", ready)
        shutil.copytree(source / "languages", ready / "languages")
        previous = temporary / "previous"
        if destination.exists() or destination.is_symlink():
            destination.rename(previous)
        try:
            ready.rename(destination)
        except OSError:
            if previous.exists() or previous.is_symlink():
                previous.rename(destination)
            raise
    print(f"Installed Sanim Zed extension: {destination}")
    print('Enable languages.Sanim.semantic_tokens="full" and inlay_hints.enabled=true in Zed settings.')


if __name__ == "__main__":
    main()
