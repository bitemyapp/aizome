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
| Moving a 320 × 200 window, 240 steps | 225 | 0.40 | 0.53 | 1.13 | 16.4 | 0.74 / 3.07 |
| A client drawing every frame (800 × 500) | 152 | 0.87 | 2.18 | 3.71 | 4.57 | 0.65 / 3.65 |
| Switching Desks (slides), 12 | 24 | 0.37 | 0.47 | 0.64 | 0.64 | 0.47 / 3.64 |
| Overview open and close, 8 | 32 | 0.72 | 2.68 | 12.9 | 12.9 | 1.73 / 5.76 |
| 10 windows, one moving | 110 | 0.70 | 0.99 | 1.56 | 2.32 | 0.54 / 6.54 |
| A full-screen client drawing every frame | 145 | 1.87 | 2.93 | 5.04 | 5.54 | 1.43 / 3.40 |

pixman:

| Scenario | Frames | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| Idle, 2 s | 0 | – | – | – | – |
| Moving a 320 × 200 window, 240 steps | 226 | 0.49 | 0.69 | 0.99 | 1.10 |
| A client drawing every frame (800 × 500) | 166 | 1.31 | 1.46 | 1.82 | 2.15 |
| Switching Desks (slides), 12 | 26 | 0.53 | 0.89 | 2.29 | 2.29 |
| Overview open and close, 8 | 32 | 7.13 | 28.7 | 57.3 | 57.3 |
| 10 windows, one moving | 114 | 0.80 | 0.89 | 1.16 | 1.44 |
| A full-screen client drawing every frame | 88 | 4.41 | 11.6 | 18.1 | 18.1 |

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
   client redrawing every frame: p50 4.4 ms, p95 11.6 ms). Each frame
   composites 5.2 million pixels on the CPU; the cost is memory traffic,
   not the scene. GLES does the same frame in 1.9 ms. Full-screen video or
   games without a usable GPU miss the budget; ordinary windows (partial
   damage) don't.
2. **The Desks overview under pixman** (p50 7 ms, p99 57 ms). Opening and
   closing it damages the whole output, and its thumbnails are window
   buffers scaled down at composition, which pixman filters on the CPU. It
   redraws only when opened, closed, filtered or dragged in, never
   continuously, so the cost is a handful of frames per use.
3. **First frames.** A frame that first shows a label (a new title, the
   overview's captions, the first cursor image) rasterizes it with
   Sidestep's offscreen canvas and uploads it; the first use of a font
   also loads it. The 12.9 ms overview p99 and the 16 ms move maximum
   under GLES are such frames. Labels are cached per text, style and
   scale, so these are one-off costs.
4. **GPU time** under headless GLES is the wait for the frame's fence,
   which includes queueing behind other GPU work; its p99 spikes (3–6 ms)
   don't hold the compositor's CPU.

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

Live, under headless sway at 2× with `SIDESTEP_FRAME_STATS=1`, the world
clock idling: a frame a second, total p50 0.14 ms (the first frame, the
whole window, took 5.4 ms; that's opening, not a tick).

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
