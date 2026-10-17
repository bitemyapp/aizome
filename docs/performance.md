# Performance

The budget is 4 ms of CPU from an event to its frame, for a typical
interaction (see `docs/architecture.md` and the kit's guide). Each app
measures its own typical interactions here: headlessly through the kit's
`Harness` (update, view, layout, diff and rasterising the damage through
the offscreen renderer), and live with `SIDESTEP_FRAME_STATS=1` (Sidestep's
per-window summary every five seconds: record, rasterise, present). Numbers
are from this laptop, debug builds unless said (dependencies at
`opt-level = 2`, the workspace's own crates unoptimised), so they're upper
bounds. Exceptions to the budget are written up with their reasons.

## Soroban (Calculator)

A key press, typed or clicked: the calculator's update, the view (up to 66
keys in Programmer mode), layout and diff, and rasterising what changed
(the display and the key that flashes, 10% of the window; with ↩ also the
tape).

| Mode | p50 | p95 | max | |
|---|---|---|---|---|
| Basic | 0.25 ms | 0.61 ms | 0.66 ms | `tests/soroban.rs`, `a_key_press_fits_the_frame_budget`, at 2×, 252 presses |
| Scientific | 0.55 ms | 0.98 ms | 1.04 ms | ″ |
| Programmer | 0.85 ms | 1.35 ms | 1.65 ms | ″ (the bit toggles redraw when the value changes) |

Live, under headless sway (pixman) at 1×, typing with `wtype`, Sidestep's
frame stats: total p50 0.43–0.48 ms, p95 0.62–0.94 ms, max 1.6 ms. The
average damage there (64%) is `wtype`'s doing: each run of it brings a new
virtual keyboard, so the window loses and regains the keyboard, and the
widget that holds the focus for typing spans the window. A real keyboard
redraws only the display and the key.

No exceptions.

## The compositor

The compositor's budget is **4 ms of CPU per output frame**: collecting
the scene's elements, rendering what's damaged and submitting it.

### Method

- The compositor times every frame it draws on every output, from the
  start of element collection to the end of submission (the damage
  tracker's render, or DrmCompositor's render and queue). Redraws that find
  nothing damaged are counted as skipped and draw nothing. Under headless
  GLES it also waits on the frame's fence and records that wait as the GPU
  time.
- The last 4096 frames per output are kept; `aizomectl stats` (IPC
  `stats`) prints p50, p95, p99, max and mean, frames rendered and frames
  skipped, and `--reset` starts over.
- `crates/aizome-comp/examples/frame_bench.rs` starts a headless
  compositor with one 2880 × 1800 output at scale 2 (the laptop panel),
  opens sidestep-ui windows (`examples/sidestep_client.rs`) and runs the
  scenarios below, resetting the statistics around each:

  ```sh
  nix develop --command cargo build --release -p aizome-comp --bin aizome --examples
  nix develop --command target/release/examples/frame_bench --renderer gles
  nix develop --command target/release/examples/frame_bench --renderer pixman
  ```

- Release build (thin LTO), Intel Core Ultra 7 365 (8 threads), Mesa
  Intel graphics (Panther Lake) through EGL on the render node; pixman on
  the same CPU. Effects tier Lean (hard shadows, 80 ms motion).

### Results (2026-10-17)

Milliseconds of compositor CPU per rendered frame.

GLES:

| Scenario | Frames | p50 | p95 | p99 | max | GPU p50 / p99 |
|---|---|---|---|---|---|---|
| Idle, 2 s | 0 | – | – | – | – | – |
| Moving a 320 × 200 window, 240 steps | 229 | 0.39 | 0.70 | 1.38 | 1.85 | 0.45 / 4.31 |
| A client drawing every frame (800 × 500) | 145 | 0.83 | 1.89 | 3.32 | 3.51 | 1.39 / 17.4 |
| Switching Desks (slides), 12 | 25 | 0.38 | 0.97 | 3.28 | 3.28 | 0.46 / 13.3 |
| Overview open and close, 8 | 32 | 1.36 | 4.70 | 13.2 | 13.2 | 1.40 / 4.57 |
| 10 windows, one moving | 110 | 0.70 | 1.39 | 3.17 | 3.76 | 0.42 / 3.51 |
| A full-screen client drawing every frame | 143 | 1.84 | 3.27 | 5.03 | 7.71 | 1.52 / 4.83 |

pixman:

| Scenario | Frames | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| Idle, 2 s | 0 | – | – | – | – |
| Moving a 320 × 200 window, 240 steps | 230 | 0.57 | 0.68 | 0.78 | 0.97 |
| A client drawing every frame (800 × 500) | 163 | 1.49 | 2.10 | 3.99 | 6.34 |
| Switching Desks (slides), 12 | 25 | 0.65 | 1.49 | 1.50 | 1.50 |
| Overview open and close, 8 | 31 | 3.15 | 12.8 | 18.5 | 18.5 |
| 10 windows, one moving | 113 | 0.73 | 1.04 | 1.22 | 1.58 |
| A full-screen client drawing every frame | 149 | 3.51 | 4.45 | 5.08 | 5.64 |

(With Archivo and IBM Plex Mono installed through `AIZOME_FONT_DIR`, as in
a session. An earlier run without them, before fonts were loaded at
start-up, had a 16 ms first frame when a window first moved: loading the
font.)

What these show:

- **Idle means no frames**: nothing changes, nothing is drawn, and no
  frame callbacks fire.
- **Moving a window composites**: the window's buffers are reused and only
  the old and new rectangles are redrawn; the client is never asked to
  redraw. Tabs and frames are cached solid-colour elements with stable
  identities, and labels are cached textures, so they cost nothing until
  they change.
- Everything ordinary stays well inside the budget on both renderers,
  pixman included. (A Desk slide here is two or three drawn frames: the
  80 ms ease-out carries a window off the output within a few frames,
  after which nothing visible changes.)

### Exceptions

1. **Full-output damage under pixman at 2880 × 1800** (a full-screen
   client redrawing every frame: p50 3.5 ms, p95 4.4 ms, max 5.6 ms; an
   earlier run on a busier machine had p95 11.6 ms). Each frame composites
   5.2 million pixels on the CPU; the cost is memory traffic, not the
   scene. GLES does the same frame in 1.8 ms. Full-screen video or games
   without a usable GPU sit at or over the budget; ordinary windows
   (partial damage) don't.
2. **The Desks overview under pixman** (p50 3.1 ms, p95 12.8 ms, max
   18.5 ms; up to 57 ms in the earlier run). Opening and closing it
   damages the whole output, and its thumbnails are window buffers scaled
   down at composition, which pixman filters on the CPU. It redraws only
   when opened, closed, filtered or dragged in, never continuously, so the
   cost is a handful of frames per use.
3. **First frames of new labels.** A frame that first shows a label (a new
   title, the overview's captions) rasterizes it with Sidestep's offscreen
   canvas and uploads it: the 13.2 ms overview maximum under GLES is its
   first opening, setting a dozen captions. Labels are cached per text,
   style and scale (and dropped half a minute after they were last
   drawn), so these are one-off costs; fonts load at start-up.
4. **GPU time** under headless GLES is the wait for the frame's fence,
   which includes queueing behind other GPU work; its p99 spikes don't
   hold the compositor's CPU.

On the udev backend frames are paced by vblank: a frame waits for the
previous one's scan-out rather than being dropped, and the CPU cost per
frame is the same as above (DrmCompositor also puts suitable buffers on
planes for direct scan-out, which only lowers it).

## Tokei (Clock)

A clock's tick is the typical interaction: once a second the program
reads the time, views the window again and diffs it, and only the digits
that changed redraw, because every character of a clock is its own
keyed element in tabular IBM Plex Mono (`widgets::digits`): 21:42:08 →
21:42:09 damages one 31 × 69pt cell, and a new minute the big clock's
last four cells and each city's minute (under 8% of the window; checked
by `a_tick_redraws_only_the_digits_that_changed`).

| Interaction | p50 | p95 | max | |
|---|---|---|---|---|
| World clock tick (4 cities) | 0.22 ms | 0.41 ms | 0.63 ms | `tests/tokei.rs`, `a_tick_fits_the_frame_budget`, at 2×, 117 ticks |
| Stopwatch frame (25 a second, while it runs and shows) | 0.16 ms | 0.28 ms | 0.32 ms | ″, 97 frames |

Live, under headless sway at 2× with `SIDESTEP_FRAME_STATS=1` (driven with
`wtype`: ❖4, Space, L): the stopwatch running, 133 frames in five seconds,
total p50 0.08 ms, p95 0.14 ms, max 1.8 ms, redrawing 3% of the window
per frame on average; the world clock idling, a frame a second, total p50
0.14 ms (the first frame, the whole window, is opening, not a tick).

The timer's ring is drawn in a thousand steps, so it redraws when a step
passes rather than on every tick. No exceptions.

## Yukimi (Software)

Headlessly, `apps/yukimi/tests/budget.rs` (`cargo test --release -p
aizome-yukimi --test budget -- --nocapture`), on the sample system, from
the event to the damage rasterised at 2× (the damage's bounding box, so
these are upper bounds). Release build, with Aizome's fonts:

| Interaction | p50 | p95 | max | drawn |
|---|---|---|---|---|
| Scrolling Installed with the wheel | 1.48 ms | 1.55 ms | 1.57 ms | 76% |
| Typing in Discover's search | 0.50 ms | 0.74 ms | 1.15 ms | 23% |
| Opening a generation in History | 1.36 ms | 1.90 ms | 2.17 ms | 61% |
| Hovering cards and rows | 0.00 ms | 0.01 ms | 0.01 ms | 0% (no view) |
| A frame of falling snow (Overview) | 0.94 ms | 1.04 ms | 1.12 ms | 19% |
| Switching pages | 4.92 ms | 5.82 ms | 7.38 ms | 97% |

Searching itself runs on a thread of its own (the window only shows the
answer), as does reading the system and the store, working out what a
generation changed, checking for updates and making the package index;
none of it is on the interface thread.

Live, the release build under headless sway (pixman) at 2× on this
system's real store (36,000 paths), Sidestep's frame stats: with the
Overview's snow falling, 184 frames in 5 s, total p50 0.93 ms, p95
2.4 ms; while switching pages every 1.2 s (through the instance socket),
p50 1.7–2.1 ms, p95 6.8–13 ms.

Exception: **switching pages**. A new page repaints the whole content
area (and the toolbar and the sidebar's selection) at 2×. Yukimi's part,
the update, the view and the layout, is 0.15–0.65 ms of it (Discover's
catalog, 47 cards, is the most); the rest is rasterising about 3.4
million pixels, which costs Sidestep's renderer about 3.5–4.5 ms for any
full window, however simple (History, the plainest page, 3.65 ms).
Switching pages is a deliberate act a few times a minute, not something
repeated under the hand, so it is left at that rather than made to draw
less than the page that's new.

`YUKIMI_STALLS=50` prints, on standard error, every update or view that
takes longer than that many milliseconds (as the GTK Yukimi does for its
interface thread).

## Tenki (Weather)

Headlessly, `apps/tenki/tests/tenki.rs` (`cargo test --release -p
aizome-tenki --test tenki interactions -- --nocapture`), on the recorded
forecasts for Tokyo, Chicago and Reykjavík, from the event to the damage
rasterised at 2× (the damage's bounding box: upper bounds). Release build,
with Aizome's fonts, the machine busy with other builds (load 13–20):

| Interaction | p50 | p95 | max | drawn |
|---|---|---|---|---|
| Choosing another place | 2.22 ms | 3.86 ms | 9.9 ms | 91% |
| Hovering the places | 0.08 ms | 0.09 ms | 0.15 ms | 5% |
| Dragging a place along the list | 0.12 ms | 0.12 ms | 0.16 ms | 5% |
| Scrolling the hours sideways | 0.27 ms | 0.32 ms | 0.83 ms | 14% |
| The minute's tick ("Updated 13 min ago") | 0.10 ms | 0.13 ms | 0.19 ms | 0–1% |

Fetching, searching and asking GeoClue run on background threads, and
files (the cache, the places, the state) are written by a thread of their
own: replacing a file waited half a second on the busy disk when it was
done on the window's thread, which the first measurements caught.

Live, a debug build (the kit unoptimised) under Aizome's headless
compositor (GLES) at 2×, with `SIDESTEP_FRAME_STATS=1`, driven with
`aizomectl input` (❖N, typing "tokyo", Return, then ❖[ and ❖] every
0.3 s): switching places, total p50 4.2 ms and p95 6.5 ms per frame
(rasterising p50 3.0 ms; 52% of the window drawn on average over those
frames); otherwise p50 0.12–0.15 ms, p95 0.16–0.4 ms. The compositor's
own frames meanwhile: p50 0.5–1.1 ms, p95 1.2–1.9 ms.

Choosing a place is the one interaction that comes near the budget: it
replaces the whole main view (another place's every number), so it
redraws nine tenths of the window, and its worst frames are the first
time a place's strings are shaped. The release build keeps its p95 under
4 ms; the test holds it under 8 ms (as it runs beside other builds) and
everything else under 4 ms.
