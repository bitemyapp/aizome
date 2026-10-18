# The compositor

`aizome` (crate `aizome-comp`) is Aizome's Wayland compositor, on smithay
0.7: a floating, stacking window manager with Haiku tabs, tab groups,
Desks, the Desks overview, the ❖Tab switcher and the tab marking menu, the
IPC server (`docs/ipc.md`) and session start-up. This page is how to run
it, what it offers clients, and how it's built and tested.

## Running it

```sh
aizome                                   # udev on a TTY, winit inside another session
aizome --backend udev                    # DRM/KMS through libseat (aizome.service does this)
aizome --backend winit                   # a window in another session, for development
aizome --backend headless --headless-output 2880x1800@2 [--headless-output …]
aizome --renderer pixman                 # or AIZOME_RENDERER=pixman; GLES falls back to it
aizome --socket NAME --no-shell --no-xwayland --no-session -- foot
```

- **Never run winit or udev on a developer's live session.** Use the
  headless backend (`scripts/headless-session` wraps it) with
  `WAYLAND_DISPLAY` and `DISPLAY` unset; `aizomectl screenshot` shows
  what it draws.
- Logging: `AIZOME_LOG` (or `RUST_LOG`), e.g. `AIZOME_LOG=debug,smithay=info`.
- Settings: `~/.config/aizome/settings.toml` (`--config PATH` for another),
  watched once a second and applied live (theme, focus colour, effects,
  keyboard, pointer and touchpad, outputs, Desks, key bindings, Night
  Light). Keys Shitsurae keeps in the sections' extra tables are honoured
  too: `[windows] tab_double_click` (`fill`, `minimise`, `nothing`),
  `[input.pointer]` and `[input.touchpad]` `scroll_factor` (scales axis
  events, wheel and finger), `[input.keyboard]` `sticky_keys` (a modifier
  tapped alone latches for the next key; tapped again it lets go) and
  `slow_keys` with `slow_keys_ms` (a key counts once held that long; a
  brush is dropped), and `[performance] renderer` (at the next start).
- Fonts: Archivo, IBM Plex Mono and Noto Sans Symbols 2 from fontconfig,
  else from `$AIZOME_FONT_DIR` or `../share/aizome/fonts` beside the
  binary.
- `AIZOME_DRM_DEVICE` picks the primary GPU on udev; `AIZOME_RENDER_NODE`
  the render node for headless GLES.

## Session

`aizome-session` (from `session/aizome.desktop`) starts `aizome.service` in
the user's systemd manager (or runs the compositor in a D-Bus session
without one). Once its Wayland socket and IPC are up the compositor:

- imports `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_CURRENT_DESKTOP=Aizome`,
  `XDG_SESSION_DESKTOP=aizome`, `XDG_SESSION_TYPE`, `AIZOME_SOCKET` and
  `AIZOME_SHELL_SOCKET` into systemd and D-Bus activation;
- tells systemd it's ready (`READY=1` on `$NOTIFY_SOCKET`) and starts
  `aizome-session.target` (bound to `graphical-session.target`);
- starts `xwayland-satellite :N` (a free display, exported as `DISPLAY`)
  and `aizome-shell` when they're on PATH, restarting them with backoff
  (1 s doubling to 30 s, reset after a minute's run);
- runs the command after `--`, if any.

Every program it starts (`spawn`, ❖Return, the shell) gets that
environment.

Under `aizome.service` it feeds systemd's watchdog (`WatchdogSec=10s`)
from its main loop; a loop stalled that long is stopped (SIGABRT, with a
core dump) and `aizome-session` starts it again, and a watcher thread logs
the main thread's backtrace to `~/.local/state/aizome/stall.log` first.
The service weighs `CPUWeight=1000`/`IOWeight=1000` within
`session.slice`. See `docs/recovery.md`.

## Recovery

**Ctrl+Alt+Del** opens the recovery overlay, checked before every other
key handling (bindings, grabs, shortcut inhibitors, the lock): the apps
with CPU, memory and whether they answer pings, Force Quit, Close, Reset
Input, Restart Shell, Reload Aizome, Lock, Log Out and Text Console. It
shows over full-screen clients and the lock screen (safe actions only
there) and has the keyboard and pointer while open. `docs/recovery.md`
has the details, the stall watchdog and the SysRq last resort.

Grabs end on release: the compositor gives the seat the release of every
press it gave it, whatever overlay came up between, and its own grabs
(moves, resizes, tab slides, the boxes) end when their button comes up or
is pressed again, whatever else is held and whatever any client does.

## What clients get

Globals: wl_compositor (6), wl_subcompositor, wl_shm, linux-dmabuf (GLES;
with feedback), wl_output (4) and xdg-output, xdg-shell, xdg-decoration
(server-side by default; clients may insist on client-side and then get
the tab only), xdg-dialog, xdg-activation, xdg-foreign, wlr-layer-shell,
ext-session-lock, wl_seat (keyboard, pointer, touch), data-device,
primary-selection, wlr and ext data-control, viewporter,
fractional-scale, presentation-time, single-pixel-buffer, cursor-shape,
idle-notify, idle-inhibit, keyboard-shortcuts-inhibit, pointer-constraints,
relative-pointer, pointer-gestures, text-input-v3, input-method-v2,
virtual-keyboard, security-context, content-type, tablet, alpha-modifier,
fifo, commit-timing, xdg-toplevel-icon, xdg-system-bell (rung as the
shell's `bell` binding), ext-foreign-toplevel-list,
wlr-foreign-toplevel-management (3), wlr-output-management (4),
wlr-screencopy (3) and wlr-gamma-control (1). Privileged ones (layer shell,
session lock, data control, input method, virtual keyboard, screencopy,
foreign toplevel management, output management, foreign toplevel list,
gamma control) aren't offered
inside a security context.

Notes for client authors:

- Outputs use integer scales (2 for panels over ~180 dpi) unless the
  settings give a fractional one.
- wl_shm takes every format the renderer reads (ABGR8888 and XBGR8888
  besides ARGB and XRGB). Under GLES a shm buffer is released as soon as
  its commit has been uploaded into a texture, not at the next commit, so
  a client can draw into one buffer again at once (smithay adds a second
  release when the next commit replaces it); under pixman, which draws
  from the client's memory, it is kept until then.
- Drag and drop: wl_data_device_manager 3 (source and offer actions),
  and the drag icon moves with its wl_surface.offset (or attach x, y)
  relative to the pointer; it shows above everything, in screenshots
  too.
- Frame callbacks fire after a frame that showed the surface, and roughly
  once a second for surfaces that don't show (other Desks, minimised).
  Nothing draws while nothing changes.
- A toplevel's app_id should be set before its first commit; it picks the
  remembered size and place, the icon, and whether ❖ reaches it untouched
  (`org.aizome.…` apps) or as Ctrl.
- Layer surfaces: `aizome-deskbar` is placed around (new windows open clear
  of it). Exclusive keyboard interactivity on the top and overlay layers
  takes the keyboard while mapped; on-demand takes it on a click; a
  grabbing popup has it until it closes, then it goes back (the order is
  in `docs/ipc.md`).
- Popups (menus) of windows and layer surfaces show above every window
  and layer surface, newest on top, anywhere on their parent's output.
- xdg-activation: a token made from an input event of the client with the
  keyboard or pointer focus moves focus; any other request marks the
  window urgent (IPC `urgent`) instead.
- ext-session-lock: `locked` is sent once every output has drawn a frame
  without the desktop; lock surfaces get the output's size and all input.
  A locker that dies leaves the session locked.

## Displays

- **On and off**: `output-config` with `enabled` (or `enabled = false` in an
  `[[outputs]]` entry, or wlr-output-management) takes an output out of the
  layout and, on udev, disables its CRTC; it stays listed (`enabled:
  false`) to be turned on again. Windows on it move to the main display.
- **Mirroring**: `mirror = "eDP-1"` in an `[[outputs]]` entry (or
  `mirror-output`) shows another output's picture, rendered afresh and
  scaled to fit with black bars, pointer included; the mirror has no place
  in the layout and no wl_output. It costs a second render of the source
  and a read-back per frame, and redraws when the source does.
- **Main display**: `primary = true` (or `set-primary-output`); else the
  first output. The pointer starts in its middle, and windows from outputs
  that go move there; the IPC reports it for the shell's Deskbar.
- **Night Light**: `[night_light]` `enabled`, `temperature` (K, default
  4000), `schedule` (`sunset` from `latitude` and `longitude`, 19:00–07:00
  without them; `manual` from `from` to `to`, `"HH:MM"`; `always`),
  checked every minute and on changes; `night-light` over IPC changes it
  for the session. It goes through the CRTC's gamma LUT (legacy gamma
  ioctl), the same under GLES and pixman, and re-applies after a VT
  switch; outputs without gamma (headless, winit, virtio-gpu) can't show
  it, and the IPC says which can. Screenshots are untinted. A
  wlr-gamma-control client (gammastep, wlsunset) holding an output's ramps
  takes it over from Night Light until it lets go; outputs without gamma
  answer it `failed`.
- **Renderer**: `[performance] renderer` (`auto`, `gpu`, `software`) picks
  GLES or pixman at start-up unless `--renderer` or `AIZOME_RENDERER` say
  otherwise; `stats` reports when it differs from the running one.

## Window management in brief

- **Placement**: centred on the usable area (inside exclusive zones),
  beside the Deskbar, cascading off windows already there; dialogs centre
  on their parent; apps reopen where they last were
  (`~/.local/state/aizome/windows.json`).
- **Tabs**: as wide as their titles (at most 360pt, and 32pt short of the
  frame); paths lose their middle, other titles their end. Click focuses
  and raises; drag moves; ⇧-drag slides the tab along the top; a double
  click or the zoom box fills; the close box closes; middle-click sends to
  the back; right-click opens the marking menu.
- **Frame**: edges and corners resize, 6pt beyond the edge; ⌃❖-drag moves
  and ⌃❖-right-drag resizes from anywhere in a window.
- **Stacks**: drag a tab past another window's tab (a dashed slot and
  "Release to stack with ‹title›" show) to stack; click a background tab
  to show it, drag it away to tear it out (⌥-drag tears out the showing
  one); ⌥❖S stacks with the window behind; IPC `stack`/`unstack`.
- **Desks**: four by default (settings `[[desks]]`), global across
  outputs; sticky windows show on all; switching slides over the motion
  time (none in Austere or with reduced motion).
- **Overview** (⌃❖↑), **switcher** (❖Tab), **marking menu** (right press
  on a tab), **area screenshots** (❖⇧4): see `docs/ipc.md` for their keys.

## Inside

| Module | What |
|---|---|
| `wm/` | The model: windows, stacking, focus history, tab groups, Desks, placement, fill/split/full screen. Plain Rust, unit-tested. |
| `decor.rs` | Tab and frame layout, hit testing, and solid-colour pieces with stable identities. |
| `render.rs` | The scene per output, front to back, and thumbnails. |
| `text.rs`, `icons.rs`, `cursor.rs` | Labels (Sidestep's offscreen canvas), app icons (desktop files, icon themes, resvg), xcursor. |
| `overlays/` | Stack-drop feedback, marking menu, switcher, overview, area picker. |
| `input.rs`, `grabs.rs`, `bindings.rs` | Input from libinput, winit or IPC; move, resize, tab slide and box grabs; the binding table and ❖ translation. |
| `shell.rs`, `handlers.rs` | Surfaces, xdg-shell, layer shell, session lock and the smaller protocols. |
| `protocols/` | wlr-screencopy, wlr foreign toplevel management, wlr output management. |
| `backend/` | headless (GLES on a render node or pixman, 60 Hz pacing only while damaged), winit, udev (DRM/KMS, multi-GPU GLES or pixman dumb buffers). |
| `ipc.rs`, `requests.rs` | The IPC socket and its requests. |
| `session.rs`, `idle.rs`, `config.rs` | Start-up, supervision, idle events, live settings. |
| `recovery/` | Ctrl+Alt+Del's overlay: state, keys and actions (`mod.rs`), drawing (`view.rs`, with the overlays' painter), apps' processes on a thread (`procinfo.rs`, `x11.rs`), pings (`ping.rs`), Reset Input (`reset.rs`). It reaches the rest through a few hooks: the first check in `decide_key` and `pointer_button`, `pointer_moved` and `scroll` in input, `may_constrain` (pointer constraints), the first elements in `render::output_elements`, `frame_flags` (udev), `exclusive_keyboard_target`, `handle_request`, `client_pong` and `constraint_requested`, and `start` in `main`. With live reload it belongs in the stable core. |
| `watchdog.rs` | systemd's watchdog keepalive and the stall watcher (backtrace, `stall.log`, the notice after a restart). |
| `reload.rs` | `reload_compositor`, Reload Aizome's entry point (live reload fills it in). |

## Tests

```sh
nix develop --command cargo test -p aizome-comp
```

- Unit tests: the model (placement, focus order, stacks, Desks), tab
  layout and hit testing, bindings, EDID parsing, statistics.
- `tests/headless.rs`: a headless compositor in a child process with
  solid-colour Wayland test clients, checking IPC state and screenshot
  pixels (the focused tab, the rail, the frame, the 3pt hard shadow,
  idle tabs, stacks, Desks, clicks, drags, resizing, the close box,
  fill, client-side decorations, idle frames, live settings, remembered
  places, ❖⇧3), under GLES and pixman; and grim, wlr-randr and lswt when
  installed.
- `tests/overlays.rs`: the switcher, the overview and the marking menu.
- `tests/keys.rs`: ❖ as Ctrl, Ctrl+Shift in terminals, ❖ for kit apps,
  system and shell chords.
- `tests/settings.rs`: tab double-click, scroll factor, sticky and slow
  keys.
- `tests/displays.rs`: outputs off and on (from IPC and the settings),
  mirroring scaled and letterboxed, the main display, Night Light's IPC
  and schedule, the renderer setting.
- `tests/protocols.rs`: wl_shm formats, shm buffers released after upload
  (GLES) or kept (pixman), drag and drop with an icon and actions.
- `tests/chromium.rs`: a window opened the way Chromium and Electron
  open theirs (an xdg-toplevel-icon set and destroyed before its buffer,
  either decoration mode, activation with their own token), and the icon
  protocol's errors.
- `tests/focus.rs`: a shell-like wayland-client (layer surfaces and their
  popups): exclusive and on-demand keyboard interactivity, grabbing menus
  getting the keys and giving them back, menus closing for an exclusive
  surface or a click elsewhere, and the popup layer's stacking.
- `tests/sidestep.rs`: a sidestep-ui client (`examples/sidestep_client`):
  server-side decorations, frame callbacks on the content subsurface, a
  layer-shell panel's exclusive zone, the session lock.
- `tests/recovery.rs`: Ctrl+Alt+Del over a full-screen client holding a
  keyboard-shortcuts inhibitor, Force Quit killing a client in a process
  of its own (and never the compositor's parent), Not responding, the
  lock screen's safe actions, Reset Input ending a stuck grab and freeing
  a locked pointer, and drags that end on release (after an overlay took
  a release, with a frozen client, and a client's late move request).

Screenshots the tests keep are in `target/tmp/aizome-shots/`; examples
are in `docs/screenshots/`. Frame times: `docs/performance.md`.

The udev backend is tested in a VM: `crates/aizome-comp/vm/udev.nix`
boots NixOS with a virtio GPU without 3D, logs a user in on tty1, starts a
development build there (libseat through logind, libinput, KMS), and
checks the output, ❖Return opening foot, typing into it and the frame
statistics; `docs/screenshots/compositor-udev-vm.png` is its scan-out.
Without a hardware GPU the backend renders with pixman into dumb buffers.
