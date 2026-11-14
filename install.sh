#!/usr/bin/env bash
# Installs Rajo for the current user (no sudo).
#   ./install.sh            install or update
#   ./install.sh --default  also make Rajo the default image viewer
set -euo pipefail
cd "$(dirname "$0")"

APP_ID=io.github.rajnish.Rajo
PREFIX="${PREFIX:-$HOME/.local}"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor"

cargo install --path . --root "$PREFIX" --locked

install -Dm644 "data/$APP_ID.svg" "$ICONS/scalable/apps/$APP_ID.svg"
mkdir -p "$APPS"
sed "s|^Exec=rajo|Exec=$PREFIX/bin/rajo|" "data/$APP_ID.desktop" > "$APPS/$APP_ID.desktop"
chmod 644 "$APPS/$APP_ID.desktop"

update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -q -t -f "$ICONS" 2>/dev/null || true

if [[ "${1:-}" == "--default" ]]; then
    mimes=$(sed -n 's/^MimeType=//p' "data/$APP_ID.desktop" | tr ';' ' ')
    # shellcheck disable=SC2086
    xdg-mime default "$APP_ID.desktop" $mimes
    echo "Rajo is now the default image viewer."
fi

echo "Installed: $PREFIX/bin/rajo"
