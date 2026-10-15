#!/bin/sh
set -eu
# Run a current uninstaller without upgrading or overwriting the user's install.
litecom_temp="$(mktemp -d)"
trap 'rm -rf "$litecom_temp"' EXIT HUP INT TERM
curl --proto '=https' --tlsv1.2 -fsSL 'https://github.com/BerriAI/litecom/releases/download/v@VERSION@/litecom-installer.sh' -o "$litecom_temp/install.sh"
LITECOM_INSTALL_DIR="$litecom_temp/bin" LITECOM_UNMANAGED_INSTALL="$litecom_temp/bin" sh "$litecom_temp/install.sh"
"$litecom_temp/bin/litecom" uninstall "$@"
