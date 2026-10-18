//! Frecency: how often and how recently something was picked, so the
//! palette learns that "term" means foot and "set" means Settings.
//!
//! Each pick adds a visit; a visit's weight halves every [`HALF_LIFE`], so
//! last week's habit fades in a month. The store is a small JSON file in the
//! state directory (`$XDG_STATE_HOME/aizome/frecency.json`), capped at
//! [`CAPACITY`] entries, the faintest dropped first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long a visit takes to count half as much.
pub const HALF_LIFE: f64 = 7.0 * 24.0 * 3600.0;
/// Most entries kept.
pub const CAPACITY: usize = 2000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    /// The decayed score at `at`.
    score: f64,
    /// Seconds since the epoch of the last update.
    at: f64,
}

/// Picks by key: `"app:org.aizome.Shitsurae"`, `"file:/home/…/notes.md"`,
/// `"cmd:org.aizome.Sumi:Edit/Find/Find and Replace…"`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Frecency {
    entries: HashMap<String, Entry>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn decayed(entry: &Entry, at: f64) -> f64 {
    entry.score * 0.5f64.powf(((at - entry.at).max(0.0)) / HALF_LIFE)
}

impl Frecency {
    /// The default store's path.
    pub fn default_path() -> PathBuf {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        state.join("aizome/frecency.json")
    }

    /// Loads the store, or starts an empty one if it's missing or unreadable.
    pub fn load(path: impl AsRef<Path>) -> Frecency {
        let path = path.as_ref();
        let mut store: Frecency =
            std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        store.path = Some(path.to_owned());
        store
    }

    /// Writes the store back where it was loaded from, atomically.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, path)
    }

    /// Records a pick of `key` now.
    pub fn visit(&mut self, key: &str) {
        self.visit_at(key, now());
    }

    pub fn visit_at(&mut self, key: &str, at: f64) {
        let entry = self.entries.entry(key.to_owned()).or_default();
        entry.score = decayed(entry, at) + 1.0;
        entry.at = at;
        if self.entries.len() > CAPACITY {
            let mut scores: Vec<(String, f64)> =
                self.entries.iter().map(|(k, e)| (k.clone(), decayed(e, at))).collect();
            scores.sort_by(|a, b| a.1.total_cmp(&b.1));
            for (k, _) in scores.into_iter().take(self.entries.len() - CAPACITY) {
                self.entries.remove(&k);
            }
        }
    }

    /// The key's current score: about the number of picks in the last
    /// half-life, 0.0 if never picked.
    pub fn score(&self, key: &str) -> f64 {
        self.score_at(key, now())
    }

    pub fn score_at(&self, key: &str, at: f64) -> f64 {
        self.entries.get(key).map_or(0.0, |e| decayed(e, at))
    }

    /// A boost to add to a match score. Matcher scores run to a few hundred
    /// for a good match, so a habit worth a few picks moves an item a few
    /// places but never lifts a poor match over a good one.
    pub fn boost(&self, key: &str) -> i64 {
        (self.score(key).ln_1p() * 24.0).round() as i64
    }

    pub fn forget(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_decay_by_half_life() {
        let mut f = Frecency::default();
        f.visit_at("app:a", 0.0);
        f.visit_at("app:a", 0.0);
        assert!((f.score_at("app:a", 0.0) - 2.0).abs() < 1e-9);
        assert!((f.score_at("app:a", HALF_LIFE) - 1.0).abs() < 1e-9);
        assert_eq!(f.score_at("app:b", 0.0), 0.0);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("aizome-frecency-{}", std::process::id()));
        let path = dir.join("f.json");
        let mut f = Frecency::load(&path);
        f.visit("file:/x");
        f.save().unwrap();
        let g = Frecency::load(&path);
        assert!(g.score("file:/x") > 0.9);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn capacity_drops_the_faintest() {
        let mut f = Frecency::default();
        for i in 0..CAPACITY {
            f.visit_at(&format!("k{i}"), i as f64);
        }
        f.visit_at("new", CAPACITY as f64);
        assert_eq!(f.entries.len(), CAPACITY);
        assert!(!f.entries.contains_key("k0"));
        assert!(f.entries.contains_key("new"));
    }
}
