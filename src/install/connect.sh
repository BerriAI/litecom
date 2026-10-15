#!/bin/sh
set -eu
# Installation is handled by cargo-dist's versioned, checksum-verifying installer.
litecom_binary="$(command -v litecom || true)"
if [ -z "$litecom_binary" ] || [ "$("$litecom_binary" --version)" != "litecom @VERSION@" ]; then
  litecom_install_dir="${LITECOM_INSTALL_DIR:-$HOME/.local/bin}"
  litecom_installer="$(mktemp)"
  trap 'rm -f "$litecom_installer"' EXIT HUP INT TERM
  curl --proto '=https' --tlsv1.2 -fsSL 'https://github.com/BerriAI/litecom/releases/download/v@VERSION@/litecom-installer.sh' -o "$litecom_installer"
  LITECOM_INSTALL_DIR="$litecom_install_dir" sh "$litecom_installer"
  litecom_binary="$litecom_install_dir/litecom"
  rm -f "$litecom_installer"
  trap - EXIT HUP INT TERM
fi
exec "$litecom_binary" "$@"
