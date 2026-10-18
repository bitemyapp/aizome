//! Reloading the compositor's logic in place, keeping every client and
//! window (the recovery overlay's "Reload Aizome").
//!
//! The live-reload work (branch `live-reload`) fills this in; until then
//! it says so.

use crate::state::State;

/// Reloads the compositor's logic; says what happened.
pub fn reload_compositor(_state: &mut State) -> Result<String, String> {
    Err("not available yet".into())
}
