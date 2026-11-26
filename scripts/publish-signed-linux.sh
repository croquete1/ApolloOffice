#!/usr/bin/env bash
# Publicação manual do ApolloOffice Linux x64. Não utiliza GitHub Actions.
set -euo pipefail

repo="croquete1/ApolloOffice"
key="$APOLLO_SIGNING_KEY_FILE"
[[ -f "$key" && ! -L "$key" ]] || { echo "Falta a chave privada Ed25519"; exit 1; }
[[ "$(stat -c %a "$key")" = "600" ]] || { echo "Exija chmod 600 para a chave privada"; exit 1; }
[[ "$(uname -s)" = "Linux" && "$(uname -m)" = "x86_64" ]] || { echo "Apenas Linux x86_64"; exit 1; }
for executable in git gh cargo python3 node openssl; do command -v "$executable" >/dev/null || exit 1; done
# The client trust key must be supplied independently, never derived as authority from the signing key.
trusted_key="$(python3 - "$key" "${APOLLO_OFFICE_UPDATE_PUBKEY:-}" <<'PYKEY'
import re, subprocess, sys
expected = sys.argv[2].strip().lower()
if not re.fullmatch(r"[0-9a-f]{64}", expected):
    raise SystemExit("Configure APOLLO_OFFICE_UPDATE_PUBKEY com a chave pública Ed25519 de confiança dos clientes")
result = subprocess.run(["openssl", "pkey", "-in", sys.argv[1], "-pubout", "-outform", "DER"],
                        capture_output=True, stdin=subprocess.DEVNULL)
prefix = bytes.fromhex("302a300506032b6570032100")
if result.returncode or len(result.stdout) != 44 or not result.stdout.startswith(prefix):
    raise SystemExit("Chave privada Ed25519 inválida ou indisponível")
if result.stdout[len(prefix):].hex() != expected:
    raise SystemExit("A chave de assinatura não corresponde à chave de confiança dos clientes")
print(expected)
PYKEY
)"
[[ -f Cargo.toml && -f src/upd.rs ]] || { echo "Execute na raiz do fork"; exit 1; }
[[ "$(git branch --show-current)" = main ]] || { echo "Publicar apenas a main"; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo "Working tree não está limpo"; exit 1; }
sha="$(git rev-parse HEAD)"
[[ "$sha" = "$(git ls-remote origin refs/heads/main | awk '{print $1}')" ]] || { echo "HEAD diferente do remoto"; exit 1; }
version="$(python3 - <<'PY'
import tomllib
with open('Cargo.toml', 'rb') as f:
    print(tomllib.load(f)['package']['version'])
PY
)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "É exigida uma versão semver estável"; exit 1; }
tag="fork-v$version"
[[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ]] || { echo "Tag já existe"; exit 1; }
# 'fork-v*' não ativa o workflow upstream configurado para tags 'v*'.
python3 quality/browser_smoke.py
python3 -m unittest discover -s quality -p 'test_*.py'
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
node --test quality/*.test.mjs
cargo build --release --locked
[[ "$sha" = "$(git rev-parse HEAD)" ]] || exit 1
[[ -z "$(git status --porcelain)" ]] || { echo "Validação alterou a árvore de trabalho"; exit 1; }
[[ "$sha" = "$(git ls-remote origin refs/heads/main | awk '{print $1}')" ]] || exit 1
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
install -m 755 target/release/apollo-office "$out/apollo-office-linux-x64"
python3 - "$out" "$version" <<'PY'
import hashlib, json, pathlib, sys
dest = pathlib.Path(sys.argv[1])
file = dest / 'apollo-office-linux-x64'
m = {'v':sys.argv[2],'files':{file.name:hashlib.sha256(file.read_bytes()).hexdigest()}}
(dest/'latest.json').write_bytes(json.dumps(m,sort_keys=True,separators=(',',':')).encode())
PY
openssl pkeyutl -sign -inkey "$key" -rawin -in "$out/latest.json" -out "$out/latest.sig.bin"
base64 -w0 "$out/latest.sig.bin" > "$out/latest.json.sig"
# Recheck the actual signed bytes against the independent trust key before GitHub writes.
python3 - "$out" "$trusted_key" <<'PYVERIFY'
from pathlib import Path
import sys
from scripts.release_manifest import verify_trusted_signature
verify_trusted_signature(Path(sys.argv[1]), sys.argv[2])
PYVERIFY
gh release create "$tag" -R "$repo" --target "$sha" --title "Apollo Office fork $version" --latest \
  --notes "Compilação Linux x64 da main $sha. Manifesto Ed25519 próprio do fork." \
  "$out/apollo-office-linux-x64" "$out/latest.json" "$out/latest.json.sig"
echo "RELEASE_PUBLICADA=$tag"
echo "APOLLO_OFFICE_UPDATE_URL=https://github.com/$repo/releases/latest/download"
echo "APOLLO_OFFICE_UPDATE_PUBKEY=$trusted_key"
echo "A chave privada permaneceu exclusivamente no computador local."
