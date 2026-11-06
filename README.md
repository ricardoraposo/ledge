# Ledge

Sticky notes that wait as a row of colored tabs on the edge of your screen. Move the cursor to the edge, pick a note, write, and it slides back out of the way.

Written in Rust with GTK 4, libadwaita and gtk4-layer-shell for Wayland. One binary, no network access, no telemetry.

Ledge is inspired by [KeepNote](https://github.com/colletpedro/keepnote) for macOS by Pedro Collet, and its sync folder reads and writes the same `.hmnote` files, so notes move between the two apps. The code is new; the paper colors and the file format come from KeepNote, under its MIT license (`LICENSE.upstream`).

## Requirements

A Wayland compositor with `wlr-layer-shell`: sway, Hyprland, KDE Plasma, niri, river, labwc or Wayfire. GNOME does not have it, so the edge deck cannot appear there.

To build it you need Rust (`cargo`) and the development files of GTK 4, libadwaita, gtk4-layer-shell and libsecret:

```bash
# Fedora
sudo dnf install cargo gtk4-devel libadwaita-devel gtk4-layer-shell-devel libsecret-devel
# Debian / Ubuntu
sudo apt install cargo libgtk-4-dev libadwaita-1-dev libgtk4-layer-shell-dev libsecret-1-dev
# Arch
sudo pacman -S rust gtk4 libadwaita gtk4-layer-shell libsecret
```

## Install

```bash
./install.sh                 # builds the release binary, puts it in ~/.local/bin, adds the menu entry and icon
./install.sh --autostart     # also starts it at login (XDG autostart; sway needs dex or similar)
./install.sh --sway          # also writes the global shortcuts to ~/.config/sway/config.d/ledge
ledge                        # start it now
```

Run `./install.sh` again after `git pull` to rebuild. `./uninstall.sh` removes everything except your notes.

## Commands

Wayland does not let an app grab global keys, so the compositor runs `ledge <command>` for each shortcut. The app keeps one instance; each command goes to the running one, and starts it if needed.

| Command | What it does | Suggested key |
|---|---|---|
| `ledge new [text]` | Make a note and open it | Super+Alt+N |
| `ledge all` | All Notes | Super+Alt+A |
| `ledge archive` | The Archive | Super+Alt+E |
| `ledge daily` | Today's daily note | Super+Alt+Y |
| `ledge focus` | Give the deck the keyboard: arrows, Return, Space, Esc | Super+Ctrl+Alt+D |
| `ledge toggle` | Fan the deck out or fold it | |
| `ledge settings` | Settings | |
| `ledge backup` | Back up now | |
| `ledge quit` | Quit | |
| `ledge install-sway` | Write the suggested keys into sway's config | |

Hyprland example: `bind = SUPER ALT, N, exec, ledge new`.

## What it does

- **Deck on the screen edge**, right or left. Hovering the stripe fans the tabs out, each with its color and a vertical label; the tab under the cursor grows out of the edge. Touching a tab shows a quick look without taking focus; clicking opens the note.
- **Docked notes** slide out of the edge and close on Esc, Ctrl+W or a click anywhere else.
- **Floating notes** (Ctrl+Alt+P or the button on the spine) are real windows: Ledge tells sway and Hyprland to float them, keep them on every workspace and drop the border. Move them by the spine, resize from the corner.
- **Markdown as you type**: headings, bold, italic, strikethrough, `==highlight==`, code, links, quotes, tables, dividers, bulleted, numbered and checklist items. Syntax hides on lines you are not editing. Lists continue on Return, Tab and Shift+Tab indent, numbers renumber, Ctrl+Return ticks a task, and a click ticks a checkbox.
- **Images and files**: paste a screenshot (Ctrl+V after `grim -g "$(slurp)" - | wl-copy`, or any copied image), drag one in, or use Insert Image in Tools. Images show in the text at the note's width; double-click opens one in your viewer; Backspace on the line below removes it. Other files become links that open with their app. Up to 25 MB each, encrypted like the text.
- **Groups**, including a Daily group. Drag a tab onto another to reorder, or onto a group to move it there.
- **Pin to Center** (up to five), **Keep on Deck**, and **archive by time** after 7, 14 or 30 days without opening. Notes are never deleted by a rule.
- **Daily notes** with a template, and yesterday's unfinished tasks brought along.
- **Delete with undo** for 10 seconds (adjustable).
- **All Notes**: search title, tags and text, filter by group or tag, sort by edited, created or title, edit in place. The sidebar button gives a focus mode.
- **Backups** once a day, seven kept; Settings has Back Up Now and Restore.
- **Sync through a folder**: one plain-text `.hmnote` file per note, the same format KeepNote writes on macOS. Point Syncthing, Nextcloud or Dropbox at it to share notes between Linux and Mac.
- **Import and export**: Markdown with attachments, plain text, and folders of `.hmnote` or `.md` files.

## Shortcuts in a note

| Keys | Action |
|---|---|
| Esc, Ctrl+W | Close |
| Ctrl+F, Ctrl+G, Shift+Ctrl+G | Find, next, previous |
| Ctrl+Alt+C, Shift+Ctrl+Alt+C | Next, previous color |
| Ctrl+Alt+P | Float, return to the deck |
| Ctrl+Shift+E | Archive |
| Ctrl+Shift+Backspace | Delete (undoable) |
| Ctrl+Return | Tick the task |
| Ctrl+B, Ctrl+I, Ctrl+Shift+X, Ctrl+Shift+H, Ctrl+E, Ctrl+K | Bold, italic, strike, highlight, code, link |
| Ctrl+Alt+1/2/3 | Heading 1, 2, 3 |
| Ctrl+Shift+7 (or 8), Ctrl+Shift+9, Ctrl+Shift+L | Bulleted, numbered, checklist |
| Ctrl+Shift+B, Ctrl+Alt+K, Ctrl+Alt+T, Ctrl+Alt+R | Quote, code block, table, divider |
| Ctrl+Shift+D | Today's date |

## Privacy and storage

- Notes live in `~/.local/share/ledge/notes.sqlite`, backups in `Backups/` next to it, settings in `~/.config/ledge/settings.json`. If you used the earlier KeepNote for Linux, Ledge keeps opening its `keepnote` folders and its keyring key.
- Note text and attachments are encrypted with AES-GCM. The key is kept in your keyring through the Secret Service (GNOME Keyring, KWallet, KeePassXC). With no keyring, Ledge falls back to a key file next to the database, readable only by you.
- Titles and tags are stored in the clear so they can be searched. Synced `.hmnote` files are plain text, like on the Mac.
- If your notes are encrypted and Ledge cannot find their key (a locked keyring, a deleted key file), it stops with a message instead of making a new key that could not open them.

## Development

```bash
cargo test
LEDGE_HOME=/tmp/ledge LEDGE_DEBUG=1 cargo run   # separate data, debug log
```

## Not ported yet

Link mentions, copy with images, the `.hmnotearchive` format, a shortcut recorder, and translations.
