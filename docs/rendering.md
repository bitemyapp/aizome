# Rendering: GPU and software

Aizome draws in two places, and both have a GPU path and a software path.

| | GPU | Software |
|---|---|---|
| The compositor (window frames, tabs, composition, the overview, the switcher) | smithay's GLES renderer on the GPU | smithay's pixman renderer on the CPU |
| The shell and the apps | — | Sidestep's rasterizer (tiny-skia) on the CPU, into shared memory |

Clients always rasterize on the CPU: Sidestep draws only what changed, into
the shared-memory buffer the compositor shows, in parallel bands for large
changes. So an app looks and performs the same whichever renderer the
compositor uses, and the choice is the compositor's alone.

## Choosing

The compositor picks GLES and falls back to pixman by itself when there is
no usable GPU: no EGL display on the render node, no GBM, or a KMS device
without acceleration (a virtual machine's virtio-gpu without virgl, a
server's BMC framebuffer, a broken driver). It says which in its log ("udev
backend, pixman renderer") and in `aizomectl stats`.

To force one:

- Settings › Performance › Renderer: Automatic, GPU or Software (the
  `[performance] renderer` key: `auto`, `gpu`, `software`). It applies at the
  next login; Settings and `aizomectl stats` say so.
- `AIZOME_RENDERER=gles|pixman`, or `aizome --renderer gles|pixman`, for one
  run.

## What each mode does

Under GLES, client buffers are uploaded to textures when they are committed
and released straight away (so Sidestep draws the next frame into the same
buffer), damage tracking repaints only what changed, and window moves are
pure composition. Under pixman, the compositor composites client memory
directly, damage-tracked the same way; a buffer is released when the next one
replaces it.

Both scan out through KMS on real displays (udev backend, with libseat), and
both render offscreen for the headless backend that tests and screenshots
use.

## Measured

Release builds, 2880×1800 at scale 2 on this laptop (Intel), CPU time per
frame the compositor renders (see `performance.md` for method and the full
tables):

| | GLES p50 / p95 | pixman p50 / p95 |
|---|---|---|
| Moving a window | 0.34 / 0.67 ms | 0.54 / 0.84 ms |
| A client drawing every frame | 0.39 / 0.60 ms | 1.59 / 1.97 ms |
| A full-screen client | 0.38 / 0.58 ms | 2.53 / 3.55 ms |
| The Desks overview | 0.92 / 2.33 ms | 3.77 / 14.8 ms |

In the NixOS VM test (virtio-gpu, no 3D, so pixman on KMS at 1440×900), a
whole session — login, terminal, Settings, Calculator, Files, lock and unlock
— rendered at p50 0.4–0.6 ms and p95 1.4–3.8 ms.

## Differences and limits in software mode

- **The Desks overview** scales every window's thumbnail on the CPU: p95
  14.8 ms at 2880×1800. It is the one thing in pixman mode outside the 4 ms
  budget; it is an occasional, deliberate view, and its first frame matters
  less than the session's. Recorded as an exception in `performance.md`.
- **Night Light** uses the display's gamma table (CRTC GAMMA_LUT), which works
  in both modes on real hardware; a display without a gamma table (most
  virtual GPUs, headless outputs) can't show it, and Settings hides the
  switch there.
- **Mirroring** reads the source output back on the CPU in both modes.
- **Effects** cost little in either mode: Aizome's look is hard shadows,
  opaque surfaces and motion that moves layers, so nothing is blurred per
  frame. The Full effects tier (not the default) makes the shell's menus and
  Deskbar translucent with a fine grain, which the compositor blends; true
  background blur is future work (it needs a compositor effect, GLES only).

Nothing was given up for software rendering: every feature of the desktop
works under pixman, and the VM test runs the whole session that way.
