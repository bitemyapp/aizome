//! Aizome's system services: what the shell (menu bar widgets, Deskbar, OSD,
//! notifications, lock screen, polkit dialog) and the apps (Shitsurae,
//! Tansu, Kura, Myaku and the rest) ask of the system.
//!
//! # Services
//!
//! Each D-Bus service is a handle built with a [`Wake`] callback:
//!
//! - **`snapshot()`** returns the current state as an `Arc`, cheaply, from a
//!   cache that the daemon's signals keep current (one background thread
//!   per service follows them);
//! - **`generation()`** counts changes, and the wake is called on each one,
//!   so the UI redraws when the counter moves; **`on_change`** adds more
//!   wakes;
//! - **action methods** return a [`Task`] at once and do the work on a
//!   thread; the wake is called again when the task is done, and its
//!   [`Result`] can then be read;
//! - **`available()`** says whether the daemon is there. Every service
//!   degrades: no BlueZ means `Bluetooth::available() == false` and an
//!   empty snapshot, never an error at start or a panic.
//!
//! The crate doesn't depend on any toolkit: the wake is any
//! `Fn() + Send + Sync`, such as a closure around a `sidestep_ui::Proxy`.
//! Every D-Bus call has a timeout (2 seconds for queries; see [`bus`]).
//!
//! Services that only read the kernel or files ([`sysinfo`], [`apps`],
//! [`keyboard`]) are plain functions and samplers the UI calls when it
//! wants to.
//!
//! # Modules
//!
//! The modules are listed in each module's own documentation, with what
//! the system must provide (daemons, NixOS options) for them to work.

pub mod audio;
pub mod bluetooth;
pub mod bus;
pub mod error;
pub mod network;
mod oneshot;
mod service;

pub use bus::Bus;
pub use error::{Error, Result};
pub use service::{Task, Wake};
