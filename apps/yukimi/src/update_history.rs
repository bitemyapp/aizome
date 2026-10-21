//! The actual revision changes from successful updates, kept across launches
//! independently of update checks. A failed build must never replace these.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use yukimi_config::lock::{FlakeLock, Input, Locked, Movement};

use crate::ops::Step;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    before: Locked,
    after: Locked,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    configurations: BTreeMap<String, BTreeMap<String, Change>>,
}

impl History {
    pub fn load(dir: &Path) -> Self {
        std::fs::read(dir.join("update-history.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn record(&mut self, of: &str, before: &[Input], after: &[Input]) {
        let changes = self.configurations.entry(of.to_owned()).or_default();
        for input in after {
            let Some(now) = input.locked.as_ref() else { continue };
            let Some(was) = before.iter().find(|i| i.name == input.name).and_then(|i| i.locked.as_ref()) else {
                continue;
            };
            if was.rev != now.rev {
                changes.insert(input.name.clone(), Change { before: was.clone(), after: now.clone() });
            }
        }
    }

    fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temporary = dir.join(format!("update-history-{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, dir.join("update-history.json"))
    }

    pub fn last(&self, of: &str, input: &Input) -> Option<&Change> {
        let change = self.configurations.get(of)?.get(&input.name)?;
        let current = input.locked.as_ref()?;
        (same_repo(current, &change.after) && current.rev == change.after.rev).then_some(change)
    }
}

fn same_repo(a: &Locked, b: &Locked) -> bool {
    a.kind == b.kind && a.host == b.host && a.owner == b.owner && a.repo == b.repo && a.url == b.url
}

/// A visible label and a tooltip that name both endpoints of the jump.
pub struct Link {
    pub label: &'static str,
    pub tooltip: String,
    pub url: String,
}

pub fn link(input: &Input, movement: Option<&Movement>, last: Option<&Change>) -> Option<Link> {
    let pinned = input.locked.as_ref()?;
    if let Some(Movement::Newer(target)) = movement
        && let Some(url) = target.rev.as_deref().and_then(|rev| pinned.compare_url(rev))
    {
        return Some(Link {
            label: "Changes",
            tooltip: format!("Pending update: pinned {} → target {}", pinned.rev.as_deref()?, target.rev.as_deref()?),
            url,
        });
    }
    if let Some(change) = last
        && same_repo(&change.before, &change.after)
        && let Some(url) = change.after.rev.as_deref().and_then(|rev| change.before.compare_url(rev))
    {
        return Some(Link {
            label: "Last update",
            tooltip: format!(
                "Last successful update: {} → {}",
                change.before.rev.as_deref()?,
                change.after.rev.as_deref()?
            ),
            url,
        });
    }
    Some(Link {
        label: "Pinned commit",
        tooltip: format!("Revision pinned in flake.lock: {}", pinned.rev.as_deref()?),
        url: pinned.web_url()?,
    })
}

fn read_inputs(dir: &Path) -> Option<Vec<Input>> {
    FlakeLock::parse(&std::fs::read_to_string(dir.join("flake.lock")).ok()?).ok().map(|lock| lock.inputs())
}

/// Snapshot before any mutation; the final step runs only if every command
/// succeeded. Read the resulting lock, since it can differ from the check.
pub fn recording_steps(
    mut steps: Vec<Step>,
    dir: std::path::PathBuf,
    of: String,
    state: std::path::PathBuf,
) -> Vec<Step> {
    let before = Arc::new(Mutex::new(None));
    let snapshot = before.clone();
    let source = dir.clone();
    steps.insert(
        0,
        Box::new(move || {
            *snapshot.lock().unwrap() = read_inputs(&source);
            Ok(None)
        }),
    );
    steps.push(Box::new(move || {
        if let (Some(before), Some(after)) = (before.lock().unwrap().as_ref(), read_inputs(&dir)) {
            let mut history = History::load(&state);
            history.record(&of, before, &after);
            if let Err(e) = history.save(&state) {
                log::warn!("Could not keep the update comparison: {e}");
            }
        }
        Ok(None)
    }));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rev: &str) -> Input {
        Input {
            name: "sekka".into(),
            locked: Some(Locked {
                kind: "github".into(),
                owner: Some("bitemyapp".into()),
                repo: Some("sekka".into()),
                rev: Some(rev.into()),
                ..Default::default()
            }),
            original: None,
            follows: None,
            follows_inputs: Vec::new(),
        }
    }

    #[test]
    fn pending_then_last_update_survives_checks_and_relaunch() {
        let dir = std::env::temp_dir().join(format!("yukimi-history-links-{}", std::process::id()));
        let (before, after) = (input("aaa"), input("bbb"));
        let target = Movement::Newer(Locked { rev: Some("bbb".into()), ..Default::default() });
        let pending = link(&before, Some(&target), None).unwrap();
        assert_eq!(pending.label, "Changes");
        assert_eq!(pending.url, "https://github.com/bitemyapp/sekka/compare/aaa..bbb");
        assert!(pending.tooltip.contains("pinned aaa → target bbb"));
        let mut history = History::default();
        history.record("flake", &[before], std::slice::from_ref(&after));
        // A no-op update must preserve the previous change.
        history.record("flake", std::slice::from_ref(&after), std::slice::from_ref(&after));
        history.save(&dir).unwrap();
        let history = History::load(&dir);
        for movement in [None, Some(&Movement::Current)] {
            let last = link(&after, movement, history.last("flake", &after)).unwrap();
            assert_eq!(last.label, "Last update");
            assert_eq!(last.url, pending.url);
        }
        assert!(history.last("other-flake", &after).is_none());
        assert!(history.last("flake", &input("ccc")).is_none());
        let mut other = after.clone();
        other.locked.as_mut().unwrap().repo = Some("elsewhere".into());
        assert!(history.last("flake", &other).is_none());
        let next = Movement::Newer(Locked { rev: Some("ccc".into()), ..Default::default() });
        assert!(link(&after, Some(&next), history.last("flake", &after)).unwrap().url.ends_with("bbb..ccc"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commit_fallback_and_unsupported_sources() {
        let pinned = input("aaa");
        let link = link(&pinned, Some(&Movement::Newer(Locked::default())), None).unwrap();
        assert_eq!(link.label, "Pinned commit");
        assert!(link.url.ends_with("/commit/aaa"));
        let mut unsupported = pinned.clone();
        unsupported.locked.as_mut().unwrap().kind = "path".into();
        assert!(super::link(&unsupported, None, None).is_none());
        assert!(pinned.locked.as_ref().unwrap().compare_url("aaa").is_none());
        let mut history = History::default();
        let mut other = input("bbb");
        other.locked.as_mut().unwrap().repo = Some("elsewhere".into());
        history.record("flake", &[pinned], std::slice::from_ref(&other));
        assert_eq!(super::link(&other, None, history.last("flake", &other)).unwrap().label, "Pinned commit");
    }

    #[test]
    fn completed_update_is_visible_without_an_update_check() {
        use crate::app::{Clock, Msg, Page, Runner, Source, Yukimi};
        use aizome_kit::testing::Harness;

        let mut model = crate::fixture::model();
        let after = model.inputs.iter().find(|i| i.name == "sekka").unwrap().clone();
        let mut before = after.clone();
        before.locked.as_mut().unwrap().rev = Some("previous".into());
        model.update_history.record(&crate::checks::subject(&model), &[before], &[after]);
        let mut app = Yukimi::with(
            Source::Sample(Arc::new(model)),
            Runner::Mock(Arc::new(|_| Vec::new())),
            Clock::Fixed(crate::fixture::NOW, crate::fixture::HOUR),
        );
        app.auto_check = false;
        let mut harness = Harness::new(app);
        let window = harness.window();
        harness.send(Msg::Show(Page::Updates));
        assert!(harness.texts(window).iter().any(|t| t == "Last update"));
        if let Some(dir) = std::env::var_os("YUKIMI_PICTURES") {
            std::fs::create_dir_all(&dir).unwrap();
            harness.save_png(window, std::path::PathBuf::from(dir).join("updates-completed.png"), 1.0);
        }
    }

    #[test]
    fn only_successful_jobs_record_the_actual_lock() {
        let dir = std::env::temp_dir().join(format!("yukimi-history-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = |rev: &str| {
            serde_json::json!({
                "root": "root", "nodes": {
                    "root": {"inputs": {"sekka": "sekka"}},
                    "sekka": {"locked": input(rev).locked}
                }
            })
            .to_string()
        };
        for success in [true, false] {
            std::fs::write(dir.join("flake.lock"), lock("aaa")).unwrap();
            let path = dir.join("flake.lock");
            let updated = lock(if success { "bbb" } else { "ccc" });
            let step: Step = Box::new(move || {
                std::fs::write(path, updated).unwrap();
                if success { Ok(None) } else { Err("build failed".into()) }
            });
            let steps = recording_steps(vec![step], dir.clone(), "flake".into(), dir.clone());
            crate::job::work(steps, Default::default(), |_| {}, |out| out);
            let history = History::load(&dir);
            assert!(history.last("flake", &input("bbb")).is_some());
            assert!(history.last("flake", &input("ccc")).is_none());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
