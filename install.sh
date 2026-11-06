#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin_dir="${HOME}/.local/bin"
apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons_dir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
autostart_dir="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is missing. Install Rust from https://rustup.rs or your distribution (dnf install cargo)."
  exit 1
fi

missing=()
for module in gtk4 libadwaita-1 gtk4-layer-shell-0 libsecret-1; do
  pkg-config --exists "$module" 2>/dev/null || missing+=("$module")
done
if ((${#missing[@]})); then
  echo "Missing development files for: ${missing[*]}"
  echo "  Fedora:        sudo dnf install gtk4-devel libadwaita-devel gtk4-layer-shell-devel libsecret-devel"
  echo "  Debian/Ubuntu: sudo apt install libgtk-4-dev libadwaita-1-dev libgtk4-layer-shell-dev libsecret-1-dev"
  echo "  Arch:          sudo pacman -S gtk4 libadwaita gtk4-layer-shell libsecret"
  exit 1
fi

jobs="${LEDGE_BUILD_JOBS:-$(( $(nproc) > 4 ? 4 : $(nproc) ))}"
(cd "$repo" && cargo build --release --locked -j "$jobs")

mkdir -p "$bin_dir" "$apps_dir" "$icons_dir"
rm -f "$bin_dir/ledge"
install -m 755 "$repo/target/release/ledge" "$bin_dir/ledge"
install -m 644 "$repo/data/io.github.ricardoraposo.Ledge.desktop" "$apps_dir/io.github.ricardoraposo.Ledge.desktop"
install -m 644 "$repo/data/io.github.ricardoraposo.Ledge.svg" "$icons_dir/io.github.ricardoraposo.Ledge.svg"
echo "Installed ledge -> $bin_dir/ledge"

if [[ " $* " == *" --autostart "* ]]; then
  mkdir -p "$autostart_dir"
  install -m 644 "$repo/data/ledge-autostart.desktop" "$autostart_dir/io.github.ricardoraposo.Ledge.desktop"
  echo "Starts at login ($autostart_dir/io.github.ricardoraposo.Ledge.desktop)"
fi

if [[ " $* " == *" --sway "* ]]; then
  "$bin_dir/ledge" install-sway
fi

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "Add $bin_dir to PATH to run ledge from anywhere." ;;
esac
