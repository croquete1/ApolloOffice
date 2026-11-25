#!/usr/bin/env bash
# Publish only a complete, newly created draft; never replace released assets.
set -euo pipefail
dist=${1:?staged release directory required}
tag=${GITHUB_REF_NAME:?release tag required}
repo=${GITHUB_REPOSITORY:?repository required}
python3 "$(dirname "$0")/release_manifest.py" --verify-trusted "$dist"

# A successful complete listing is required, including drafts. Never interpret
# an API or authorization failure as proof that a tag has not been released.
gh api --paginate "repos/$repo/releases?per_page=100" |
  jq -se --arg tag "$tag" 'all(.[][]; .tag_name != $tag)' >/dev/null || {
    echo "Cannot prove release tag is unused; refusing to publish." >&2
    exit 1
  }

args=(--repo "$repo" --draft --verify-tag --title "$tag")
if [[ "$tag" == *-* ]]; then args+=(--prerelease); fi
# gh creates the draft and uploads the assets without making it public.
# A failed upload leaves a draft for manual inspection, not a retry overwrite.
gh release create "$tag" "${args[@]}" "$dist"/apollo-office-* "$dist/latest.json" "$dist/latest.json.sig"
verified=$(mktemp -d)
trap 'rm -rf -- "$verified"' EXIT
gh release view "$tag" --repo "$repo" --json isDraft --jq '.isDraft' | grep -qx true
gh release download "$tag" --repo "$repo" --pattern '*' --dir "$verified"
diff -qr -- "$dist" "$verified"
python3 "$(dirname "$0")/release_manifest.py" --verify-trusted "$verified"
gh release edit "$tag" --repo "$repo" --draft=false
