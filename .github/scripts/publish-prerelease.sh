#!/usr/bin/env bash
set -euo pipefail

[[ "$#" -gt 0 ]] || { echo "Release assets are required" >&2; exit 1; }

short_sha="${GITHUB_SHA:0:12}"
timestamp="$(TZ=UTC git show -s --format=%cd --date=format-local:%Y%m%dT%H%M%SZ "${GITHUB_SHA}")"
release_date="$(TZ=UTC git show -s --format=%cd --date='format-local:%Y-%m-%d %H:%M:%S UTC' "${GITHUB_SHA}")"
tag="prerelease-${timestamp}-${short_sha}"
release_notes="${RUNNER_TEMP}/prerelease.md"
cat > "${release_notes}" <<EOF
Automated build of commit [\`${short_sha}\`](${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/commit/${GITHUB_SHA}), committed ${release_date}.

Platform assets are added as their builds finish. GitHub displays the SHA-256 checksum beside each download.

- **Windows x86-64:** Extract \`shrimply-windows-x86_64.zip\` and run \`shrimply-qt.exe\`.
- **macOS Apple Silicon:** Unzip \`shrimply-macos-arm64.zip\` and move \`Shrimply.app\` to Applications. The app is ad-hoc signed; approve its first launch in System Settings > Privacy & Security if macOS blocks it.
- **Linux Flatpak:** Install the attached \`shrimply-gtk-x86_64.flatpak\` with \`flatpak install --user ./shrimply-gtk-x86_64.flatpak\`, or use the updating remote:

\`\`\`console
flatpak install --user https://soirihiroka.github.io/shrimply/shrimply-prerelease.flatpakref
flatpak update --user dev.shrimply.Shrimply
\`\`\`
EOF

if ! gh release view "${tag}" >/dev/null 2>&1; then
  if gh release create "${tag}" "$@" \
    --target "${GITHUB_SHA}" \
    --title "Shrimply Prerelease ${release_date} (${short_sha})" \
    --notes-file "${release_notes}" \
    --prerelease --latest=false; then
    exit 0
  fi
  # Another platform may have created the shared release in the meantime.
  gh release view "${tag}" >/dev/null
fi

assets="$(gh release view "${tag}" --json assets --jq '.assets[].name')"
existing_assets="$(mktemp -d "${RUNNER_TEMP}/existing-prerelease.XXXXXX")"
trap 'rm -rf "${existing_assets}"' EXIT
for asset in "$@"; do
  name="$(basename "${asset}")"
  if grep -Fxq -- "${name}" <<< "${assets}"; then
    gh release download "${tag}" --dir "${existing_assets}" --pattern "${name}"
    cmp "${asset}" "${existing_assets}/${name}"
  else
    gh release upload "${tag}" "${asset}"
  fi
done
