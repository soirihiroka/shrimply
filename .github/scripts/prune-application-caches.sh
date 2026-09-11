#!/usr/bin/env bash
set -euo pipefail

cache_key="${1:?application cache key is required}"
shift

# Cache saves can report success without uploading (for example, when storage is
# unavailable). Confirm the exact replacement exists before deleting anything.
saved_keys="$(gh cache list --ref refs/heads/main --key "${cache_key}" --json key --jq '.[].key')"
if ! grep -Fxq -- "${cache_key}" <<< "${saved_keys}"; then
  echo "::notice::Replacement application cache is missing; preserving previous caches."
  exit 0
fi

for prefix in "$@"; do
  old_caches="$(gh api --paginate --method GET "repos/${GITHUB_REPOSITORY}/actions/caches" \
    -f ref=refs/heads/main -f "key=${prefix}" \
    --jq '.actions_caches[] | [.id, .key] | @tsv')"
  while IFS=$'\t' read -r cache_id old_key; do
    if [[ -n "${cache_id}" && "${old_key}" != "${cache_key}" ]]; then
      gh cache delete "${cache_id}"
    fi
  done <<< "${old_caches}"
done
