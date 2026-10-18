# The compositor's IPC

The compositor (`aizome`) listens on a Unix socket,
`$XDG_RUNTIME_DIR/aizome.$WAYLAND_DISPLAY.sock`, and exports its path to
everything it starts (and to systemd and D-Bus activation) as
`AIZOME_SOCKET`.

- Framing: newline-delimited JSON, one message per line.
- A client writes a **request** and reads one **response** back, in order.
- After `subscribe` (answered `{"response":"ok"}`) the connection carries
  only **events**, one per line, until either side closes it.
- Enums are tagged in kebab-case: `{"request":"focus","id":7}`,
  `{"response":"error","message":"no window 7"}`,
  `{"event":"window-focused","id":7}`.
- Optional fields may be left out. An optional window `id` left out means
  the focused window; an optional `state` left out toggles.

The types live in the `aizome-ipc` crate (`crates/aizome-ipc/src/lib.rs`),
with a blocking client:

```rust
let mut c = aizome_ipc::Client::connect()?;          // $AIZOME_SOCKET
let r = c.request(&aizome_ipc::Request::Windows)?;   // one round trip
for event in aizome_ipc::Client::connect()?.events()? { /* … */ }
```

`aizomectl` covers every request from the command line (`aizomectl help`;
`--json` prints raw responses).

Protocol version: **1** (`version` reports it).

## Requests

| Request | Fields | Response |
|---|---|---|
| `version` | | `version {version, protocol}` |
| `outputs` | | `outputs {outputs: [OutputInfo]}` |
| `windows` | | `windows {windows: [WindowInfo]}`, bottom of the stack first |
| `desks` | | `desks {desks: [DeskInfo], current}` (`current` is 1-based) |
| `focused-window` | | `window {window: WindowInfo?}` |
| `focus` | `id` | `ok`; shows the window wherever it is (its Desk, unminimised, its app unhidden) and focuses it |
| `close` | `id?` | `ok`; asks the window to close |
| `minimise` | `id?` | `ok` |
| `unminimise` | `id` | `ok`; also focuses it |
| `hide-app` | `app_id?` or `pid?` | `ok`; neither means the focused window's app |
| `hide-others` | | `ok`; hides every app but the focused one's |
| `show-app` | `app_id?` or `pid?` | `ok` |
| `fill` | `id?`, `state?` | `ok`; the zoom box: the usable area |
| `split` | `id?`, `side` (`left`/`right`) | `ok`; half the usable area; again on the same side restores |
| `fullscreen` | `id?`, `state?` | `ok` |
| `float` | `id?`, `state?` | `ok`; keep above other windows |
| `sticky` | `id?`, `state?` | `ok`; show on every Desk |
| `move-to-desk` | `id?`, `desk`, `follow` (bool, default false) | `ok`; moves the window's tab group too |
| `set-desk` | `desk` | `ok`; switches Desks |
| `add-desk` | `name`, `folder?`, `index?` (1-based; default last) | `ok` |
| `rename-desk` | `desk`, `name` | `ok` |
| `set-desk-folder` | `desk`, `folder?` | `ok` |
| `remove-desk` | `desk` | `ok`; its windows move to the Desk before it; the last Desk stays |
| `move-desk` | `desk`, `index` | `ok`; reorders |
| `stack` | `a`, `b` | `ok`; window `b` (and its group) joins `a`'s tab group and shows |
| `unstack` | `id?` | `ok`; out of its group, a tab's height down and right |
| `move` | `id?`, `x`, `y` | `ok`; the content's top-left corner (the tab and frame sit outside it) |
| `resize` | `id?`, `rect {x,y,w,h}` | `ok`; the content rectangle |
| `overview` | `state?` | `ok`; the Desks overview |
| `switcher` | `reverse?`, `state?` | `ok`; opens it (it stays until a pick or Esc, since no ❖ is held), `state: false` closes it |
| `spawn` | `argv`, `cwd?`, `in_desk_folder` (default true) | `spawned {pid}`; with the session's environment, in `cwd`, else the current Desk's folder, else home |
| `screenshot` | `output?`, `path?`, `window?`, `area? {x,y,w,h}`, `clipboard` (bool) | `screenshot {path}`; PNG of one output, a window (with its tab, frame and shadow), an area, or (none given) every output side by side; default path `~/Pictures/Screenshots/Screenshot YYYY-MM-DD at HH.MM.SS.png`; also emits `screenshot-taken` |
| `reload-config` | | `ok`; re-reads `settings.toml`, applies it, emits `config-changed` |
| `output-config` | `name`, `mode?` (`"WxH"` or `"WxH@Hz"`), `scale?`, `position? [x,y]`, `transform?`, `enabled?` | `ok`; for this session (Settings writes the file). `enabled: false` turns the output off (its CRTC disabled on udev; out of the layout, its windows moved to the main display); `true` turns it on again, at `position` or right of the others |
| `stats` | `reset` (bool) | `stats {stats}`: frame times per output |
| `quit` | | `ok`; ends the session |
| `power-off-monitors` / `power-on-monitors` | | `ok` |
| `lock` | | `ok`; tells the shell (a `binding` event with action `lock`) |
| `subscribe` | | `ok`, then events |
| `action` | `action` (a binding action name, below) | `ok`; as if its keys were pressed |
| `synthesize` | `events: [Synthetic]` | `ok`; input through the compositor's own paths (bindings, focus, grabs): for tests and automation |
| `mirror-output` | `name`, `source?` | `ok`; `name` shows `source`'s picture, scaled to fit and letterboxed in black, and leaves the layout (no wl_output, no windows or layer surfaces of its own); `source: null` gives it its own picture back, right of the others. For this session; Settings writes `mirror = "…"` in the output's `[[outputs]]` entry |
| `set-primary-output` | `name` | `ok`; the main display for this session (Settings writes `primary = true`) |
| `night-light` | `enabled?`, `temperature?` (Kelvin, 1900–6500) | `night-light {state: NightLightState}`, after changing what's given for this session (Settings writes `[night_light]`) |
| `pick-area` | `tag?` | `ok` at once; the screenshot picker opens (drag over an area, click for the whole display under the pointer, Space and a click for a window, Esc to give up) and its pick is sent to subscribers as `area-picked` with this tag, not taken as a screenshot. The shell uses it to choose what to record |

`desk` is a 1-based position (`2`) or a name (`"Mail"`). `transform` is
`normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180` or
`flipped-270`.

`Synthetic` events, tagged by `type`:

- `pointer-motion {x, y}`: to a point in the global layout (points);
- `pointer-button {button, pressed}`: `"left"`, `"right"`, `"middle"` or
  an evdev code;
- `click {button}`;
- `pointer-scroll {dx, dy}`;
- `key {key, pressed}`: a name (`super`, `shift`, `ctrl`, `alt`, `tab`,
  `return`, `escape`, `space`, `grave`, `up`/`down`/`left`/`right`, letters,
  digits, punctuation (`,` `.` `/` `;` `'` `[` `]` `\` `-` `=`), `f1`…`f12`,
  `print`) or an evdev code;
- `chord {keys}`: `"super+shift+3"` presses in order, releases in reverse.

`aizomectl input key NAME` and `input button NAME` press and release
(a tap); add `down` or `up` for one half. `aizomectl input type TEXT` taps
the keys for letters, digits, spaces and punctuation, with Shift where a
US layout needs it (capitals, `!` `(` `*` `+` `?` and the like).

## Responses

`ok`, `error {message}`, and the ones named above.

### WindowInfo

| Field | |
|---|---|
| `id` | stable while the window lives; never reused |
| `app_id`, `title`, `pid` | |
| `desk` | 1-based; `null` when sticky |
| `sticky` | |
| `group` | the tab group's id when stacked, else `null` |
| `group_members` | the group's windows in tab order (empty when not stacked) |
| `group_active` | whether this member is the one showing |
| `focused`, `minimised`, `hidden` (its app hidden), `urgent` (asked for attention without a valid activation token), `fullscreen`, `fill`, `float` | |
| `geometry {x,y,w,h}` | the content rectangle in the global layout |
| `output` | the output showing most of it |
| `client_side_decorations` | it draws its own title bar; Aizome adds only the tab |
| `kit_app` | app_id starts with `org.aizome.`: ❖ isn't translated for it |
| `parent` | the window it's a dialog of |

### DeskInfo

`index` (1-based), `id` (stable), `name`, `folder`, `windows` (count, not
counting sticky ones), `current`.

### OutputInfo

`name`, `make`, `model`, `serial`, `enabled`, `current_mode {width, height,
refresh (mHz), preferred}`, `modes`, `scale`, `transform`, `logical {x,y,w,h}`
(in points), `usable {x,y,w,h}` (minus layer-shell exclusive zones),
`physical_size_mm`, `powered`, `primary` (the main display: `primary = true`
in the settings or `set-primary-output`, else the first in the layout; the
Deskbar's, and where windows rescued from outputs that go end up),
`mirror` (the output whose picture it shows; `logical` and `usable` are then
the source's), `gamma` (it takes gamma ramps, so Night Light shows).
`enabled: false` outputs (turned off with `output-config` or `enabled =
false` in the settings) are listed so they can be turned on again; they
have no place in the layout.

### NightLightState

`enabled`, `temperature` (the warm temperature, K), `schedule` (`sunset`,
`manual`, `always`), `active` (warm now), `current` (what the displays show
now, K; 6500 is neutral), `outputs` (those that can show it: DRM outputs
whose CRTC takes a gamma LUT; headless and winit outputs can't).

### Stats

`renderer` (`gles`/`pixman`), `backend` (`headless`/`winit`/`udev`), and per
output: `name`, `frames_rendered`, `frames_skipped` (redraws that found no
damage), `cpu_us` and `gpu_us` (when measurable) as `{p50, p95, p99, max,
mean}` in microseconds over the last 4096 frames, `samples`.
`renderer_setting` is `[performance] renderer` (`auto`, `gpu`, `software`);
it applies at the next start, and `renderer_after_restart` says it asks for
another renderer than the one running. `screencopy`, once a client has
made a wlr-screencopy copy, is the same record for the copies (`cpu_us`
per copy: collecting the scene, rendering it into the client's buffer,
and for wl_shm reading it back).

## Events

| Event | Fields | When |
|---|---|---|
| `window-opened` | `window` | a toplevel drew its first frame |
| `window-closed` | `id` | |
| `window-changed` | `window` | title, app_id, Desk, state, geometry or group changed (coalesced per loop iteration) |
| `window-focused` | `id?` | |
| `desk-changed` | `index`, `id`, `name` | the current Desk changed |
| `desks-changed` | `desks` | added, removed, renamed, reordered or their window counts changed |
| `outputs-changed` | `outputs` | plugged, unplugged, reconfigured, or their usable area changed |
| `config-changed` | | settings were applied |
| `binding` | `action`, `pointer? [x,y]`, `output?` | a key or pointer binding that belongs to the shell |
| `screenshot-taken` | `path` | |
| `idle` | `kind` (`lock` or `monitors-off`) | no input for the configured minutes |
| `resumed` | | input after `idle` |
| `lock-state-changed` | `locked` | an ext-session-lock client locked or unlocked |
| `night-light-changed` | `state` | Night Light turned on or off, or warmed or cooled (the schedule) |
| `area-picked` | `tag`, `area?` (`{x,y,w,h}` in points; `null` when given up), `output?` (the one holding its centre), `window?` (when a window was picked) | the picker opened by `pick-area` closed |

## Bindings

The system takes only these chords (architecture.md, "Keys"); everything
else goes to the focused app. ❖ is the Logo key. Overrides go in
`settings.toml`'s `[keys.bindings]`, as `"Ctrl+Super+Up" = "overview"` (or
`"none"` to unbind, or `"spawn prog args"`).

Handled by the compositor:

| Chord | Action name |
|---|---|
| ❖Tab, ❖⇧Tab | `switcher`, `switcher-reverse` |
| ❖\` | `next-window-of-app` |
| ❖H, ❖⌥H, ❖M | `hide-app`, `hide-others`, `minimise` |
| ⌃❖1…9, ⌃⇧❖1…9 | `go-to-desk-N`, `take-to-desk-N` |
| ⌃❖←, ⌃❖→ (with ⇧: take the window) | `previous-desk`, `next-desk`, `take-to-previous-desk`, `take-to-next-desk` |
| ⌃❖↑ | `overview` |
| ⌥❖↑, ⌥❖↓, ⌥❖←, ⌥❖→ | `fill`, `minimise` (park), `split-left`, `split-right` |
| ⌥❖S | `stack-with-behind` |
| ⌃❖F | `fullscreen` |
| ❖Return | `terminal` (`keys.terminal`, default `foot`, in the Desk's folder) |
| ❖⇧3, Print | `screenshot-screen` |
| ❖⇧4 | `screenshot-area` (a rubber band; Space picks a window; Esc cancels) |
| XF86Switch_VT_N | `vt-N` |

Sent to subscribers as `binding` events, for the shell:

| Chord | `action` |
|---|---|
| ❖Space | `palette` |
| ❖⌥Space | `toolchest` |
| ❖⌥M; F10 over Aizome's apps (`org.aizome.…`) and the desktop (other apps keep F10) | `menu-bar` (focus the menu bar) |
| XF86PowerOff (the power key, when logind leaves it to the session) | `power-key` |
| a client rings xdg-system-bell | `bell`, with the `output` (its window's, else the pointer's) |
| ⌃❖Q | `lock` |
| ❖⇧5 | `screenshot-tool` |
| volume, mute, mic mute keys | `volume-up`, `volume-down`, `mute`, `mic-mute` |
| brightness keys | `brightness-up`, `brightness-down` |
| media keys | `play-pause`, `next`, `previous`, `stop` |
| ❖-right-click | `app-menu`, with `pointer` and `output` |

Pointer bindings handled by the compositor: ⌃❖-drag moves the window under
the pointer, ⌃❖-right-drag resizes it from the nearest corner.

## Layer-shell namespaces

The compositor treats these namespaces specially:

- `aizome-deskbar`: new windows are placed beside it (it isn't an exclusive
  zone, but windows open clear of it).

Keyboard focus, highest first:

1. the lock surface, while locked;
2. a mapped layer surface on the overlay or top layer with Exclusive
   keyboard interactivity (the topmost, on the active output first): it
   keeps the keys while it shows, whatever gets focused meanwhile, and
   closes menus (grabbing popups) of other surfaces as it maps; a menu of
   another surface asked for while it shows is dismissed at once;
3. a grabbing popup (an `xdg_popup.grab`, of a window or a layer surface,
   including a layer surface's popup made with a null parent and
   `get_popup`): the topmost of the chain has the keys until it closes,
   then they go back to what had them before (the focused window, say,
   not the bar the menu came from). Any serial is accepted for the grab,
   so a menu opened for a `binding` event (which carries no Wayland
   serial) can grab with the client's last serial, or 0. A press outside
   the client, or focus moving to another window, closes the menus (and
   the press then does what it does there);
4. an OnDemand layer surface, once clicked, until a window is clicked or
   it unmaps;
5. the focused window.

Popups of every window and layer surface are drawn above all windows and
layer surfaces (a menu of the menu bar shows over the Deskbar and over
overlay surfaces), newest on top, and take the pointer there first. They
are constrained to their parent's output by the positioner's constraint
adjustments, not to the parent's area.
