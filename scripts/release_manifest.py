"""Generate the signed update manifest only for a complete release inventory."""
import hashlib
import json
import os
from pathlib import Path

EXPECTED_ASSETS = frozenset({
    "apollo-office-windows-x64.exe",
    "apollo-office-windows-arm64.exe",
    "apollo-office-linux-x64",
    "apollo-office-linux-arm64",
    "apollo-office-macos-arm64",
    "apollo-office-macos-x64",
})


def manifest(directory: Path, tag: str) -> dict:
    if not tag.startswith("v") or len(tag) == 1:
        raise ValueError("invalid release tag")
    assets = {p.name for p in directory.glob("apollo-office-*")}
    if assets != EXPECTED_ASSETS:
        raise ValueError(f"invalid release inventory: missing={sorted(EXPECTED_ASSETS - assets)}, unexpected={sorted(assets - EXPECTED_ASSETS)}")
    files = {}
    for name in sorted(EXPECTED_ASSETS):
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"invalid release asset: {name}")
        with path.open("rb") as stream:
            files[name] = hashlib.file_digest(stream, "sha256").hexdigest()
    return {"v": tag[1:], "files": files}


def verify_staged(directory: Path, tag: str) -> None:
    expected = EXPECTED_ASSETS | {"latest.json", "latest.json.sig"}
    if {p.name for p in directory.iterdir()} != expected:
        raise ValueError("invalid staged release inventory")
    for name in ("latest.json", "latest.json.sig"):
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"invalid metadata: {name}")
    if json.loads((directory / "latest.json").read_text()) != manifest(directory, tag):
        raise ValueError("manifest does not match staged binaries")
    import base64
    signature = base64.b64decode((directory / "latest.json.sig").read_bytes(), validate=True)
    if len(signature) != 64:
        raise ValueError("invalid Ed25519 signature length")


if __name__ == "__main__":
    import sys
    tag = os.environ["GITHUB_REF_NAME"]
    if len(sys.argv) == 3 and sys.argv[1] == "--verify":
        verify_staged(Path(sys.argv[2]), tag)
    elif len(sys.argv) == 1:
        print(json.dumps(manifest(Path.cwd(), tag)))
    else:
        raise SystemExit("usage: release_manifest.py [--verify directory]")
