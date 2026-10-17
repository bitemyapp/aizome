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
