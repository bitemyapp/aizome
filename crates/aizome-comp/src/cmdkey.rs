//! The command key for every app: ❖-shortcuts become the Ctrl ones apps
//! that don't use Aizome's kit expect (Ctrl+Shift in terminals).

/// Aizome's own apps name themselves `org.aizome.…` and take ❖ as is.
pub fn is_kit_app(app_id: &str) -> bool {
    app_id.starts_with("org.aizome.")
}
