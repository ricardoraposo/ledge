#!/usr/bin/env bash
set -euo pipefail
ledge quit 2>/dev/null || true
rm -f "${HOME}/.local/bin/ledge"
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.github.ricardoraposo.Ledge.desktop"
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/io.github.ricardoraposo.Ledge.svg"
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/io.github.ricardoraposo.Ledge.desktop"
rm -f "${HOME}/.config/sway/config.d/ledge"
echo "Removed Ledge. Your notes stay in ${XDG_DATA_HOME:-$HOME/.local/share}/ledge; delete that folder to remove them too."
echo "If you used --sway, remove the 'include ~/.config/sway/config.d/ledge' line from your sway config."
