# Aizome's apps

Every app is a crate in `apps/<name>` built with `aizome-kit`. Names are
Japanese words that say what the app is for, in keeping with Aizome and Sekka
(Tatami, Yukimi, Genkan); each carries a plain English generic name, which
the Toolchest, the palette and the Deskbar show beside it and search by.

| App | Generic name | Crate | Binary | App ID | What the name means |
|---|---|---|---|---|---|
| Shitsurae しつらえ | Settings | `apps/shitsurae` | `aizome-shitsurae` | `org.aizome.Shitsurae` | arranging a room for its season and guests |
| Yukimi 雪見 | Software | `apps/yukimi` | `aizome-yukimi` | `org.aizome.Yukimi` | snow-viewing, through a shōji's low pane of glass (Sekka's Nix software manager, made native) |
| Hako 箱 | Files | `apps/hako` | `aizome-hako` | `org.aizome.Hako` | box |
| Sumi 墨 | Text Editor | `apps/sumi` | `aizome-sumi` | `org.aizome.Sumi` | ink |
| Fusen 付箋 | Notes | `apps/fusen` | `aizome-fusen` | `org.aizome.Fusen` | sticky notes |
| Koyomi 暦 | Calendar | `apps/koyomi` | `aizome-koyomi` | `org.aizome.Koyomi` | almanac, calendar |
| Tokei 時計 | Clock | `apps/tokei` | `aizome-tokei` | `org.aizome.Tokei` | clock |
| Soroban 算盤 | Calculator | `apps/soroban` | `aizome-soroban` | `org.aizome.Soroban` | abacus |
| Tenki 天気 | Weather | `apps/tenki` | `aizome-tenki` | `org.aizome.Tenki` | weather |
| Tansu 箪笥 | Disks | `apps/tansu` | `aizome-tansu` | `org.aizome.Tansu` | chest of drawers (partitions as drawers) |
| Kura 蔵 | Disk Usage | `apps/kura` | `aizome-kura` | `org.aizome.Kura` | storehouse |
| Myaku 脈 | Activity | `apps/myaku` | `aizome-myaku` | `org.aizome.Myaku` | pulse |

Also part of the session, not separate apps: the Toolchest, the command
palette, notifications, the lock screen and the polkit dialog (all in
`aizome-shell`). The terminal is foot, themed by Aizome's module.

## Conventions

- **Desktop entry** `org.aizome.<Name>.desktop`: `Name=<Name>`,
  `GenericName=<generic>`, `Comment`, `Keywords` (the generic name and
  common synonyms: Settings gets "preferences;control panel;system";
  Soroban gets "calculator;math;arithmetic"), `Categories`,
  `Icon=org.aizome.<Name>`, `Exec=aizome-<name> %F` (or `%U`),
  `StartupWMClass=org.aizome.<Name>`, `DBusActivatable=false`.
- **Icon**: `icons/apps/org.aizome.<Name>.svg` (drawn by `icons/genicons.py`),
  installed to `share/icons/hicolor/scalable/apps/`.
- **App ID** on every window (`xdg_toplevel.set_app_id`): the app ID above.
  The compositor treats `org.aizome.*` windows as speaking Aizome's command
  key natively.
- **One instance**: a second launch hands its arguments to the running one
  (a Unix socket in `$XDG_RUNTIME_DIR/aizome-app.<name>.sock`) and exits.
- **State** in `$XDG_STATE_HOME/aizome/<name>/`, **data** in
  `$XDG_DATA_HOME/aizome/<name>/`, **settings** in
  `$XDG_CONFIG_HOME/aizome/<name>.toml`; desktop-wide settings come from
  `aizome-config`.
- **Menus**: the kit's standard app, Edit, Window and Help menus, extended
  by the app; every command reachable from the palette.
- **Look**: only the kit's widgets and the theme's colours; Night and Day
  must both look finished; every target 24pt or more; all of it reachable
  by keyboard.
- **Frame budget**: 4 ms for a typical interaction; measure with
  `SIDESTEP_FRAME_STATS=1` and note exceptions in `docs/performance.md`.
- **Headless test**: each app has at least one test that runs it with
  `SIDESTEP_BACKEND=null`, drives it through messages and checks its state,
  and golden images of its main views through the offscreen renderer.

## What each app does

Sources to port from are in the Sekka repository (`~/work/sekka`, or
`git -C ~/work/sekka show origin/main:<path>` for what's only on main):
Tatami's GTK apps under `desktop/tatami/` and Yukimi under `apps/yukimi/`.
They are MIT OR Apache-2.0 by the same author: copy what helps and rewrite
the UI with the kit. Keep their behaviour and their care for edge cases.

### Shitsurae (Settings)

The macOS System Settings model in Aizome's look: a sidebar (search field
on top, then the user's card, then panes in groups with Lucide glyphs on
small `focus`/dye-coloured tiles) and the pane on the right as forms.
`aizome-shitsurae --pane <id>` opens a pane; `--list-settings --json`
prints every pane and searchable option (id, title, path, keywords) for the
palette. Search filters the sidebar and highlights options.

- **Network**: Wi-Fi (on/off, networks, join with password or hidden, known
  networks, forget, auto-join), wired, VPN and WireGuard (list, connect,
  import, remove), per-connection details (IPv4/IPv6 method, addresses,
  DNS, proxy read-out, metered, MAC), airplane mode, hotspot if supported.
  Port `desktop/tatami/network` (window.rs, details.rs, join.rs) onto
  `aizome-sys::network`.
- **Bluetooth**: on/off, discoverable name, nearby and paired devices,
  pairing with passkey confirmation, connect, disconnect, forget, battery.
- **Sound**: output and input devices with volume, balance and mute, level
  meter for input, per-app volumes, alert sound and volume.
- **Notifications**: Do Not Disturb with schedule, per-app allow/banners/
  sounds, show previews when locked.
- **Displays**: arrangement by dragging output rectangles, resolution and
  refresh, integer scale (1×, 2×, 3×) with fractional opt-in, rotation,
  enable/disable, mirror, primary; changes apply live through the
  compositor IPC (`output-config`) with a 15-second "Keep these settings?"
  revert. Night Light if the compositor supports it. Port the arrangement
  logic and care of `desktop/tatami/displays` (arrangement.rs, model.rs),
  replacing Hyprland calls with Aizome's IPC.
- **Appearance**: Night, Day, Auto (by sunset), High Contrast, with
  previews; focus colour (amber, cyan, rose, lime); effects tier (Full,
  Lean, Austere) with a description of each; Size (small, medium, large);
  wallpaper for Night and Day (a grid of `/run/current-system/sw/share/backgrounds`
  and `~/Pictures`); reduced motion.
- **Desktop & Deskbar**: Desks (names, folders, add/remove), Deskbar
  (collapsed, pinned apps, show the Shelf), desktop icons (show disks,
  Home, Trash), windows (stack new windows of the same app, double-click on a
  tab: fill or minimise, command key for all apps).
- **Menu Bar**: clock (24h, seconds, date), which systray widgets and meters
  show.
- **Search**: palette scopes on and off, file index folders and
  exclusions, rebuild the index, forget the palette's history.
- **Performance** (Display & Performance): the renderer (GPU or software,
  with which one is running), effects tier, frame statistics from
  `aizomectl stats` and the apps (p50/p95 against the 4 ms budget, live),
  the refresh rate.
- **Battery & Power**: power mode, battery health and a charge history,
  turn the display off after, lock after, what the power key does, sleep
  when the lid closes (shows the system configuration's value if NixOS
  manages it).
- **Lock Screen**: lock after idle, require the password after sleep or
  screen off, show the clock.
- **Users & Groups**: the current user's name, picture, password; other
  accounts (AccountsService), admin or standard, add and remove, automatic
  login.
- **Keyboard**: layouts (searchable list from `aizome-sys`'s xkb data, with
  variants), switching shortcut, key repeat delay and rate, Caps Lock as
  (Caps Lock, Control, Escape, Compose), compose key, and the list of
  Aizome's shortcuts (from the compositor's `keys` config, editable).
- **Mouse** and **Trackpad**: pointer speed and acceleration, natural
  scrolling, tap to click, two-finger or corner right-click, disable while
  typing, scroll speed.
- **Printers & Scanners**: printers with status, add (discovered or by
  address), remove, default, test page, queue.
- **General**: About (the mark, Sekka version, hardware summary, hostname
  rename), Software Update (opens Yukimi's Updates), Storage (a bar of what
  fills the disk, opening Kura), Date & Time (automatic, timezone map or
  search, 24h), Language & Region (locale, formats preview), Default Apps
  (browser, mail, terminal, text editor, files, images, PDF, music, video),
  Login Items (autostart entries on and off).
- **Accessibility**: reduced motion, High Contrast, larger text (Size),
  cursor size, sticky and slow keys (xkb options), visual alert flash.

### Yukimi (Software)

Yukimi, Sekka's Nix software manager, made native: the same Overview,
Installed, Discover, Updates, History and Storage views and the same
behaviour (read `apps/yukimi/README.md`), rebuilt with the kit. Copy the
UI-independent crates (`yukimi-config`, `yukimi-store`, `yukimi-system`)
into `apps/yukimi/crates/` and keep using Sekka's installed privileged
helper (`yukimi-helper`, with its polkit policy) for system changes, found
where Sekka's Yukimi package installs it.

As built: the helper is the program Sekka's polkit action
`io.github.bitemyapp.Yukimi.change-system` names (its
`org.freedesktop.policykit.exec.path`, read from the installed policy), run
through `/run/wrappers/bin/pkexec` with the GTK Yukimi's arguments, so it
needs `programs.yukimi.enable` (Sekka sets it on every desktop) and nothing
of Aizome's. Its cache is `~/.cache/aizome/yukimi`, reading the GTK
Yukimi's package index and last check from `~/.cache/yukimi` until it has
its own. `aizome-yukimi --sample` shows a sample system;
`AIZOME_YUKIMI_ETC=/some/etc` looks at another system's configuration
without changing anything.

### Hako (Files)

A file manager in NeXT's spirit: Miller columns as the main view (with list
and icon views as alternatives), a sidebar (Places, the Desk's folder,
disks, Recent, Trash), a path bar, Quick Look on Space (images, text,
PDF first page if cheap), search through `aizome-search`, file operations
with a progress panel (copy, move, trash, restore, delete, rename, new
folder, duplicate, compress and extract with `bsdtar`), Open With, Get Info
(size, kind, dates, permissions), drag and drop with other apps, the
Desktop and the Shelf, image thumbnails cached in
`$XDG_CACHE_HOME/thumbnails` (the freedesktop spec), hidden files toggle,
sorting. Tabs are Aizome's own tab groups, so no tabs inside.

### Sumi (Text Editor)

The board's editor: a toolbar with the language pop-up ("Markdown ⌄"), B,
I and </> buttons, and the status "Ln 48, Col 12 · UTF-8 · LF"; line numbers
in `faint`; the current line on `sunk`; Markdown styled as you type
(headings in `blue` 600, code in `green`, emphasis); syntax colouring for
common languages (Rust, Python, JS/TS, C, Nix, shell, TOML, JSON, YAML)
with a small built-in highlighter; Format menu as on the board (Font ▸,
Paragraph ▸, Bold ❖B, Italic ❖I, Code ❖E, Link… ❖K, Heading ▸, Quote,
Copy Style ⌥❖C, Paste Style ⌥❖V); Find and Replace with regex; encodings
and line endings; autosave to a sidecar and restore after a crash; recent
files; open large files quickly.

### Fusen (Notes)

Notes as Markdown files in `~/Notes` (so they're plain, searchable and
agent-readable): a list with search and pinned notes, folders, checklists,
a preview line; the editor is the kit's text editor with light Markdown
styling. Any note can float as a sticky (a small always-on-top window in
the note's colour). Takes "New Note with Selection" from the Services menu.

### Koyomi (Calendar)

Month, week and day views; events with time, all-day, location, notes,
repeat (RRULE subset), alerts; calendars as `.ics` files in
`$XDG_DATA_HOME/aizome/koyomi/`, import and export `.ics`; a small
`aizome-koyomi --agent` user service fires alerts as notifications. Week
numbers and the first day of the week from the locale.

### Tokei (Clock)

World clocks (search cities, with day/night and offset), alarms (repeat
days, label, sound; scheduled as systemd user timers so they fire when the
app is closed, and wake from sleep when asked), a timer and a stopwatch with
laps. Tabular IBM Plex Mono digits.

### Soroban (Calculator)

Basic, Scientific and Programmer (hex, dec, oct, bin; bitwise; word size)
modes; typed expressions through `aizome-search::calc`; a paper tape of the
history that can be copied or saved; keyboard first.

### Tenki (Weather)

Current conditions, hourly for 24 hours and ten days ahead from Open-Meteo
(no key needed), cities found with Open-Meteo's geocoding and the current
place through geoclue; saved places; units (metric, imperial); icons drawn
from Lucide's weather set; a gentle failure state offline.

### Tansu (Disks)

GParted's job with Aizome's care: pick a disk; a partition map bar
(segments sized by bytes, coloured by filesystem, used space shaded); the
partition table (name, filesystem, label, mount point, size, used, flags);
operations queued and listed (create, delete, format, resize, label, flags,
check, mount, unmount, create a partition table, unlock LUKS) and applied
only after a confirmation that lists them in plain words; SMART health;
images (attach `.iso`/`.img` as loop devices). Everything through UDisks2
(`aizome-sys::disks`). It never touches a disk in tests: use mocks, and
loop devices only inside the VM test.

### Kura (Disk Usage)

DaisyDisk's job: scan a disk or folder (fast, parallel, showing progress),
show it as a sunburst of rings by size with the largest items labelled,
drill in and out, a list beside it, a Collector to gather items and move
them to the Trash in one go, Nix-aware (the store's size, with a button
that opens Yukimi's Storage rather than deleting store paths). Every scan
is also written for agents: `$XDG_CACHE_HOME/aizome/kura/index.json` (a
stable, documented schema: the tree to a depth, the largest files and
folders, sizes, modified dates, kinds, and hints such as caches, build
outputs, old downloads and duplicates) and `index.md` (a readable summary of
what could be freed and how), plus `aizome-kura scan [PATH] --json` and
`aizome-kura report` on the command line. Document the format in
`docs/kura-index.md`.

### Myaku (Activity)

Port Tatami's Activity (`desktop/tatami/activity` on Sekka's main): apps
and processes with CPU, memory, energy, disk and GPU use; sorting, search,
quit and force quit; graphs of CPU, memory, disk and network over time; the
inspector for a process. Through `aizome-sys::sysinfo`.
