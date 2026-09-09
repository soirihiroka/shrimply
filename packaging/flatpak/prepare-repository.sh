#!/usr/bin/env bash
set -euo pipefail

: "${FLATPAK_REPO_DIR:?}"
: "${FLATPAK_BUNDLE:?}"
: "${FLATPAK_PAGES_DIR:?}"
: "${FLATPAK_PAGES_URL:?}"
: "${FLATPAK_GPG_PRIVATE_KEY:?Missing Flatpak signing key}"
: "${GITHUB_SHA:?}"
: "${GITHUB_REPOSITORY:?}"

app_id=dev.shrimply.Shrimply
branch=master
arch=x86_64
runtime_repo=https://dl.flathub.org/repo/flathub.flatpakrepo
repo_url="${FLATPAK_PAGES_URL%/}/repo/"
public_key="$(dirname "$0")/shrimply.asc"
test -s "${public_key}" || { echo "Missing public signing key: ${public_key}" >&2; exit 1; }

export GNUPGHOME
GNUPGHOME="$(mktemp -d)"
trap 'gpgconf --kill gpg-agent; rm -r -- "${GNUPGHOME}"' EXIT
gpg --batch --import "${public_key}"
fingerprint="$(gpg --with-colons --show-keys "${public_key}" | awk -F: '$1 == "fpr" {print $10; exit}')"
test -n "${fingerprint}"
printf '%s\n' "${FLATPAK_GPG_PRIVATE_KEY}" | gpg --batch --import
unset FLATPAK_GPG_PRIVATE_KEY

flatpak build-sign --arch="${arch}" --gpg-sign="${fingerprint}" \
  "${FLATPAK_REPO_DIR}" "${app_id}" "${branch}"
flatpak build-update-repo \
  --gpg-sign="${fingerprint}" \
  --gpg-import="${public_key}" \
  --title='Shrimply Prerelease' \
  --comment='Pre-alpha GTK builds from main' \
  --homepage="${FLATPAK_PAGES_URL%/}/" \
  --default-branch="${branch}" \
  --generate-static-deltas --prune --prune-depth=0 \
  "${FLATPAK_REPO_DIR}"
ostree --repo="${FLATPAK_REPO_DIR}" fsck

# Copy regular files: Pages artifacts cannot contain filesystem links.
mkdir -p "${FLATPAK_PAGES_DIR}/repo" "$(dirname "${FLATPAK_BUNDLE}")"
cp -rL "${FLATPAK_REPO_DIR}/." "${FLATPAK_PAGES_DIR}/repo/"
gpg --export "${fingerprint}" > "${FLATPAK_PAGES_DIR}/shrimply.gpg"
encoded_key="$(base64 --wrap=0 < "${FLATPAK_PAGES_DIR}/shrimply.gpg")"

cat > "${FLATPAK_PAGES_DIR}/shrimply-prerelease.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Shrimply Prerelease
Comment=Pre-alpha GTK builds from main
Url=${repo_url}
Homepage=${FLATPAK_PAGES_URL%/}/
DefaultBranch=${branch}
GPGKey=${encoded_key}
EOF

cat > "${FLATPAK_PAGES_DIR}/shrimply-prerelease.flatpakref" <<EOF
[Flatpak Ref]
Title=Shrimply Prerelease
Name=${app_id}
Branch=${branch}
Url=${repo_url}
SuggestRemoteName=shrimply-prerelease
Homepage=${FLATPAK_PAGES_URL%/}/
RuntimeRepo=${runtime_repo}
IsRuntime=false
GPGKey=${encoded_key}
EOF

cat > "${FLATPAK_PAGES_DIR}/index.html" <<EOF
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Shrimply Prerelease</title>
<h1>Shrimply Prerelease</h1>
<p>Pre-alpha GTK builds for x86_64 Linux, updated from main.
A compatible NVIDIA driver is required.</p>
<p><a href="shrimply-prerelease.flatpakref">Install Shrimply</a> ·
<a href="shrimply-prerelease.flatpakrepo">Add Flatpak repository</a> ·
<a href="https://github.com/${GITHUB_REPOSITORY}/releases">Download bundles</a></p>
<pre>flatpak install --user ${FLATPAK_PAGES_URL%/}/shrimply-prerelease.flatpakref
flatpak update --user ${app_id}</pre>
<p>Source: <a href="https://github.com/${GITHUB_REPOSITORY}/commit/${GITHUB_SHA}">${GITHUB_SHA}</a></p>
</html>
EOF

flatpak build-bundle \
  --arch="${arch}" \
  --repo-url="${repo_url}" \
  --gpg-keys="${FLATPAK_PAGES_DIR}/shrimply.gpg" \
  --runtime-repo="${runtime_repo}" \
  "${FLATPAK_REPO_DIR}" "${FLATPAK_BUNDLE}" "${app_id}" "${branch}"
(
  cd "$(dirname "${FLATPAK_BUNDLE}")"
  bundle_name="$(basename "${FLATPAK_BUNDLE}")"
  sha256sum "${bundle_name}" > "${bundle_name}.sha256"
  sha256sum --check "${bundle_name}.sha256"
)
