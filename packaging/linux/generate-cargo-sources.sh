#!/usr/bin/env bash
# Generates cargo-sources.json from Cargo.lock for offline Flatpak builds.
#
# Requires: python3, python3-venv, curl
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Setting up temporary Python environment..."
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet tomlkit aiohttp

echo "Downloading flatpak-cargo-generator.py..."
# Pin to an immutable commit of flatpak/flatpak-builder-tools rather than floating master.
flatpak_tools_rev="9d86d336a39b9af38b86f114b6ce5820fe260f91"
curl -fsSL "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/${flatpak_tools_rev}/cargo/flatpak-cargo-generator.py" \
  -o "$work/flatpak-cargo-generator.py"

echo "Generating cargo-sources.json from Cargo.lock..."
"$work/venv/bin/python3" "$work/flatpak-cargo-generator.py" \
  "$root/Cargo.lock" \
  -o "$here/cargo-sources.json"

echo "Wrote $here/cargo-sources.json successfully."
