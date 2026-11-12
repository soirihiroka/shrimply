"""Format declared workspace packages, never local dependency workspaces/submodules."""

import argparse
from pathlib import Path
import subprocess
import tomllib


def package_flags(manifest: Path) -> list[str]:
    root = manifest.resolve().parent
    document = tomllib.loads(manifest.read_text())
    workspace = document["workspace"]
    excluded = {
        path.resolve()
        for pattern in workspace.get("exclude", [])
        for path in root.glob(pattern)
    }
    submodules = set()
    for ancestor in (root, *root.parents):
        modules = ancestor / ".gitmodules"
        if modules.is_file():
            config = subprocess.run(
                ["git", "config", "--null", "--file", str(modules),
                 "--get-regexp", r"^submodule\..*\.path$"],
                capture_output=True, text=True, encoding="utf-8", check=False,
            )
            if config.returncode not in (0, 1):
                raise ValueError(f"cannot read submodule paths: {config.stderr.strip()}")
            submodules = {
                (ancestor / entry.split("\n", 1)[1]).resolve()
                for entry in config.stdout.split("\0") if entry
            }
            break
    members = {root} if "package" in document else set()
    for pattern in workspace["members"]:
        paths = list(root.glob(pattern))
        if not paths:
            raise ValueError(f"workspace member {pattern!r} does not exist")
        members.update(path.resolve() for path in paths)
    packages = set()
    for member in sorted(members):
        if not member.is_relative_to(root):
            raise ValueError(f"refusing to format a member outside the workspace: {member}")
        if any(member.is_relative_to(path) for path in submodules):
            raise ValueError(f"refusing to format a Git submodule: {member}")
        if any(member.is_relative_to(path) for path in excluded):
            continue
        package = tomllib.loads((member / "Cargo.toml").read_text())["package"]
        packages.add(package["name"])
    if not packages:
        raise ValueError("no declared workspace packages to format")
    return [flag for name in sorted(packages) for flag in ("--package", name)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path("Cargo.toml"))
    parser.add_argument("--check", action="store_true")
    parser.add_argument("cargo", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    cargo = args.cargo[1:] if args.cargo[:1] == ["--"] else args.cargo
    if not cargo:
        parser.error("pass the Cargo command after --")
    try:
        flags = package_flags(args.manifest)
    except (OSError, KeyError, ValueError) as error:
        parser.error(str(error))
    command = [*cargo, "fmt", "--manifest-path", str(args.manifest.resolve()), *flags]
    if args.check:
        command.append("--check")
    return subprocess.run(command, cwd=args.manifest.resolve().parent, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
