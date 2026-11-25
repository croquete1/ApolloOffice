"""Generate the signed update manifest only for a complete release inventory."""
import base64
import hashlib
import json
import os
import re
import subprocess
import tempfile
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
    signature = base64.b64decode((directory / "latest.json.sig").read_bytes(), validate=True)
    if len(signature) != 64:
        raise ValueError("invalid Ed25519 signature length")


def verify_trusted_signature(directory: Path, public_key: str) -> None:
    """Authenticate exact bytes using the same raw hex Ed25519 key as the updater."""
    public_key = public_key.strip()
    if re.fullmatch(r"[0-9a-fA-F]{64}", public_key) is None:
        raise ValueError("configure APOLLO_OFFICE_UPDATE_PUBKEY with the trusted 32-byte hex key")
    # RFC 8410 SubjectPublicKeyInfo prefix for a raw Ed25519 public key.
    der = bytes.fromhex("302a300506032b6570032100" + public_key)
    signature = base64.b64decode((directory / "latest.json.sig").read_bytes(), validate=True)
    with tempfile.TemporaryDirectory() as temp:
        key = Path(temp) / "public.der"
        sig = Path(temp) / "signature.raw"
        key.write_bytes(der)
        sig.write_bytes(signature)
        subprocess.run(
            ["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
             "-inkey", str(key), "-rawin", "-in", str(directory / "latest.json"),
             "-sigfile", str(sig)],
            check=True, capture_output=True,
        )


if __name__ == "__main__":
    import sys
    tag = os.environ["GITHUB_REF_NAME"]
    if len(sys.argv) == 3 and sys.argv[1] == "--verify":
        verify_staged(Path(sys.argv[2]), tag)
    elif len(sys.argv) == 3 and sys.argv[1] == "--verify-trusted":
        directory = Path(sys.argv[2])
        verify_staged(directory, tag)
        verify_trusted_signature(directory, os.environ.get("APOLLO_OFFICE_UPDATE_PUBKEY", ""))
    elif len(sys.argv) == 1:
        print(json.dumps(manifest(Path.cwd(), tag)))
    else:
        raise SystemExit("usage: release_manifest.py [--verify directory | --verify-trusted directory]")
