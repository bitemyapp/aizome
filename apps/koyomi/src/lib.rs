//! Koyomi 暦, Aizome's calendar.
//!
//! Calendars are iCalendar files (RFC 5545), one per calendar, in
//! `$XDG_DATA_HOME/aizome/koyomi/`. The library reads and writes them
//! ([`ics`], [`model`]), expands recurring events ([`rrule`], [`occur`])
//! in the reader's time zone ([`tz`]), and answers the shell
//! ([`upcoming`]) and the alert agent ([`agent`]); the window is
//! [`app`] and [`ui`].

pub mod ics;
pub mod model;
pub mod occur;
pub mod rrule;
pub mod tz;
pub mod vtimezone;
