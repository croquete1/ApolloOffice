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


if __name__ == "__main__":
    print(json.dumps(manifest(Path.cwd(), os.environ["GITHUB_REF_NAME"])))
