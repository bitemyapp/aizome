//! Aizome's desktop shell: one sidestep-ui process, one kit
//! [`Program`](aizome_kit::Program), drawing everything of Aizome that
//! isn't a window's frame or an app (see `docs/shell.md`).

pub mod actions;
pub mod apps;
pub mod clock;
pub mod ipc;
pub mod launch;
pub mod look;
pub mod menus;
pub mod notifications;
pub mod palette;
pub mod pam;
pub mod shelf;
pub mod systray;
pub mod wm;

/// `aizome-shell --help`.
pub const USAGE: &str = "usage: aizome-shell

Aizome's desktop shell: the menu bar, Toolchest, Desks strip, systray,
Deskbar and Shelf, command palette, menus, notifications, OSD, desktop,
lock screen and authentication dialog. The compositor starts it.

Environment:
  AIZOME_SOCKET          the compositor's IPC socket
  AIZOME_SHELL_SOCKET    where apps publish their menus to the shell
  AIZOME_FONT_DIR        fonts to install for this process
  SIDESTEP_FRAME_STATS=1 print each surface's frame times every 5 s";

/// Run the shell until the session ends: the process's exit code.
pub fn run() -> i32 {
    0
}
