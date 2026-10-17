//! Yukimi driven headlessly on the sample system: navigating, searching,
//! scrolling, and the try / install for me / install for everyone, remove,
//! update, go back and clean up flows, with stand-ins for Nix and the
//! helper that print what Nix would. Nothing here changes the machine.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aizome_kit::layout::{Point, Vec2};
use aizome_kit::testing::Harness;
use aizome_kit::WindowKey;
use aizome_yukimi::app::{Changes, Clock, Msg, Page, Runner, Source, Yukimi};
use aizome_yukimi::job::Outcome;
use aizome_yukimi::ops::{Operation, Run, Step, SystemChange};

/// What a stand-in step prints: progress on its error output, as Nix's
/// `internal-json` log, and the helper's report.
const WORKS: &str = r#"echo '@nix {"action":"start","id":1,"level":0,"type":0,"text":"Building the new system","fields":[]}' >&2
echo '@nix {"action":"start","id":2,"level":4,"type":104,"text":"","fields":[]}' >&2
echo '@nix {"action":"result","id":2,"type":105,"fields":[1,1,0,0]}' >&2
echo 'building /nix/store/0123456789abcdfghijklmnpqrsvwxyz-gimp2-2.10.38.drv' >&2
echo '{"ok":true}'"#;

const FAILS: &str = r#"echo 'building the new system' >&2
echo '@nix {"action":"msg","level":0,"msg":"error: builder for gimp2 failed with exit code 1"}' >&2
exit 1"#;

/// A step that runs `script` in place of Nix or the helper.
fn script(text: &'static str) -> Step {
    Box::new(move || {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", text]);
        Ok(Some(Run { command, as_root: false }))
    })
}

/// The sample, with every operation it runs written down and run as
/// `script`.
fn sample(script_text: &'static str) -> (Harness<Yukimi>, WindowKey, Arc<Mutex<Vec<Operation>>>) {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let seen = ran.clone();
    let model = Arc::new(aizome_yukimi::fixture::model());
    let runner = Runner::Mock(Arc::new(move |op: &Operation| {
        seen.lock().unwrap().push(op.clone());
        vec![script(script_text)]
    }));
    let mut app = Yukimi::with(Source::Sample(model.clone()), runner, Clock::Fixed(aizome_yukimi::fixture::NOW, 21));
    app.check = Some(Arc::new(aizome_yukimi::fixture::check(&model)));
    app.index = Some(Arc::new(aizome_yukimi::fixture::index()));
    let h = Harness::new(app);
    let w = h.window();
    (h, w, ran)
}

/// Let background work answer, until `done` (or five seconds).
fn settle(h: &mut Harness<Yukimi>, done: impl Fn(&Yukimi) -> bool) {
    let start = Instant::now();
    while !done(h.program()) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
        h.send(Msg::NoteGone(0));
    }
    assert!(done(h.program()), "it settled");
}

fn has(h: &Harness<Yukimi>, w: WindowKey, text: &str) -> bool {
    h.texts(w).iter().any(|t| t == text)
}

#[test]
fn the_sidebar_goes_to_each_page() {
    let (mut h, w, _) = sample(WORKS);
    assert_eq!(h.program().page, Page::Overview);
    assert!(has(&h, w, "ganymede"));
    for (title, page) in [
        ("Installed", Page::Installed),
        ("Discover", Page::Discover),
        ("Updates", Page::Updates),
        ("History", Page::History),
        ("Storage", Page::Storage),
        ("Overview", Page::Overview),
    ] {
        let at = h.semantics(w).into_iter().find(|(s, _)| s.label.as_deref() == Some(title)).map(|(_, r)| r).expect(title);
        h.click(w, at.center());
        assert_eq!(h.program().page, page, "{title}");
    }
    // The menu's shortcuts go there too.
    h.command(w, '4');
    assert_eq!(h.program().page, Page::Updates);
}

#[test]
fn searching_finds_packages_and_opens_one() {
    let (mut h, w, _) = sample(WORKS);
    h.send(Msg::Show(Page::Discover));
    // The search field takes the keyboard when Discover opens.
    h.type_text(w, "image");
    assert_eq!(h.program().query, "image");
    settle(&mut h, |a| a.found.as_ref().is_some_and(|f| f.query == "image"));
    assert!(has(&h, w, "imagemagick"));
    assert!(has(&h, w, "broken"));
    // A newer query's answer is the one shown.
    h.send(Msg::Search("gimp".into()));
    settle(&mut h, |a| a.found.as_ref().is_some_and(|f| f.query == "gimp"));
    assert!(has(&h, w, "gimp2"));
    // The catalog's application by that name comes first.
    assert!(has(&h, w, "GIMP"));
    assert!(h.click_text(w, "gimp2"));
    let details = h.program().details.as_ref().map(|(k, _)| *k).expect("the details sheet");
    assert!(has(&h, details, "GNU Image Manipulation Program, the 2.10 series"));
    assert!(has(&h, details, "Install for Everyone"));
}

#[test]
fn install_for_everyone_asks_first_then_runs_the_helper() {
    let (mut h, w, ran) = sample(WORKS);
    h.send(Msg::Show(Page::Discover));
    let gimp2 = aizome_yukimi::fixture::index().get("gimp2").cloned().unwrap();
    h.send(Msg::Open(gimp2));
    let details = h.program().details.as_ref().map(|(k, _)| *k).unwrap();
    assert!(h.click_text(details, "Install for Everyone"));
    // Nothing runs before the question is answered.
    assert!(ran.lock().unwrap().is_empty());
    assert!(has(&h, w, "Install gimp2 for everyone?"), "{:?}", h.texts(w));
    assert!(click_last(&mut h, w, "Install"));
    let expected = Operation::ChangeSystem {
        change: SystemChange {
            packages: Some(vec!["gimp".into(), "inkscape".into(), "ripgrep".into(), "gimp2".into()]),
            ..SystemChange::default()
        },
        what: "gimp2".into(),
        adding: true,
    };
    assert_eq!(ran.lock().unwrap().as_slice(), [expected.clone()]);
    // What the helper is asked, as the GTK Yukimi asks it.
    assert_eq!(
        expected.helper_args(None),
        ["change", "--packages", "gimp,inkscape,ripgrep,gimp2", "--mode", "auto"]
    );
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    let job = h.program().job.as_ref().unwrap();
    assert_eq!(job.outcome, Some(Outcome::Done("gimp2 is installed for everyone".into())));
    assert!(job.log.iter().any(|l| l.contains("gimp2-2.10.38.drv")));
    // The sheet says so a moment, then goes; the note stays in the status line.
    let sheet = h.program().job_sheet.unwrap();
    assert!(has(&h, sheet, "gimp2 is installed for everyone"));
    h.advance(Duration::from_secs(1));
    assert!(h.program().job.is_none());
    assert!(has(&h, w, "gimp2 is installed for everyone"));
}

#[test]
fn install_for_me_and_try_it_need_no_password() {
    let (mut h, _, ran) = sample(WORKS);
    let htop = aizome_yukimi::fixture::index().get("htop").cloned().unwrap();
    h.send(Msg::InstallForMe(htop.clone()));
    assert_eq!(ran.lock().unwrap().as_slice(), [Operation::InstallForMe { attr: "htop".into(), unfree: false }]);
    assert!(!ran.lock().unwrap()[0].is_system());
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    h.advance(Duration::from_secs(1));
    // Trying runs nothing of Yukimi's: a terminal opens with nix shell.
    h.send(Msg::TryIt(htop));
    assert_eq!(ran.lock().unwrap().len(), 1);
    assert!(h.program().notes.iter().any(|(_, n)| n == "Trying htop in a terminal"));
}

#[test]
fn a_failure_stays_with_its_log() {
    let (mut h, _, _) = sample(FAILS);
    h.send(Msg::Run(Operation::Rollback { generation: 26 }));
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    let sheet = h.program().job_sheet.unwrap();
    let job = h.program().job.as_ref().unwrap();
    let Some(Outcome::Failed(why)) = &job.outcome else { panic!("{:?}", job.outcome) };
    assert!(why.starts_with("error: builder for gimp2 failed"), "{why}");
    assert!(why.contains("the running system was not changed"));
    assert!(job.details, "the log shows");
    assert!(has(&h, sheet, "building the new system"));
    // It stays until closed.
    h.advance(Duration::from_secs(5));
    assert!(h.program().job.is_some());
    assert!(h.click_text(sheet, "Close"), "{:?}", h.texts(sheet));
    assert!(h.program().job.is_none());
}

#[test]
fn stopping_stops() {
    let (mut h, _, _) = sample("sleep 30");
    let started = Instant::now();
    h.send(Msg::Run(Operation::Clean { older_than_days: Some(30) }));
    std::thread::sleep(Duration::from_millis(100));
    h.send(Msg::StopJob);
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    assert!(started.elapsed() < Duration::from_secs(10));
    let Some(Outcome::Failed(why)) = &h.program().job.as_ref().unwrap().outcome else { panic!() };
    assert!(why.starts_with("Stopped."), "{why}");
}

#[test]
fn removing_takes_just_that_entry_out() {
    let (mut h, w, ran) = sample(WORKS);
    h.send(Msg::Show(Page::Installed));
    assert!(has(&h, w, "ADDED WITH YUKIMI"));
    h.send(Msg::Remove(aizome_yukimi::app::Removal::Package("ripgrep".into())));
    assert!(has(&h, w, "Remove ripgrep?"));
    assert!(click_last(&mut h, w, "Remove"));
    assert_eq!(ran.lock().unwrap().as_slice(), [Operation::ChangeSystem {
        change: SystemChange { packages: Some(vec!["gimp".into(), "inkscape".into()]), ..SystemChange::default() },
        what: "ripgrep".into(),
        adding: false,
    }]);
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    h.advance(Duration::from_secs(1));
    // An entry written in configuration.nix comes out of its list.
    h.send(Msg::Remove(aizome_yukimi::app::Removal::Listed { attr: "wget".into(), user: None }));
    assert!(click_last(&mut h, w, "Remove"));
    let last = ran.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.helper_args(None), ["change", "--unlist-system", "wget", "--mode", "auto"]);
}

#[test]
fn updating_takes_what_moves_forward() {
    let (mut h, w, ran) = sample(WORKS);
    h.send(Msg::Show(Page::Updates));
    assert!(has(&h, w, "newer version"));
    assert!(has(&h, w, "couldn't check"));
    // On a Sekka system one update takes Sekka and every newer source.
    assert!(h.click_text(w, "Update"));
    let op = ran.lock().unwrap().first().cloned().unwrap();
    assert_eq!(op, Operation::UpdateSekka { inputs: vec!["nixpkgs".into(), "sekka".into()] });
    assert_eq!(op.helper_args(None), ["change", "--sekka", "--mode", "auto", "--update", "nixpkgs,sekka"]);
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
}

#[test]
fn going_back_and_cleaning_up_ask_first() {
    let (mut h, w, ran) = sample(WORKS);
    h.send(Msg::Show(Page::History));
    h.send(Msg::ToggleGeneration(27));
    settle(&mut h, |a| matches!(a.changes.get(&27), Some(Changes::Ready { .. })));
    // What 27 changed from 26: it has its packages, and can be returned to.
    assert!(has(&h, w, "Return…"));
    assert!(h.click_text(w, "Return…"));
    assert!(has(&h, w, "Return to generation 27?"));
    assert!(click_last(&mut h, w, "Return"));
    assert_eq!(ran.lock().unwrap().as_slice(), [Operation::Rollback { generation: 27 }]);
    settle(&mut h, |a| a.job.as_ref().is_some_and(|j| j.outcome.is_some()));
    h.advance(Duration::from_secs(1));

    h.send(Msg::Show(Page::Storage));
    h.send(Msg::Older(2));
    h.send(Msg::CleanUp);
    assert!(has(&h, w, "Clean up the store?"));
    assert!(click_last(&mut h, w, "Clean up"));
    assert_eq!(ran.lock().unwrap().last(), Some(&Operation::Clean { older_than_days: Some(90) }));
}

#[test]
fn the_running_generation_shows_what_it_changed() {
    let (mut h, w, _) = sample(WORKS);
    h.send(Msg::Show(Page::History));
    h.send(Msg::ToggleGeneration(28));
    settle(&mut h, |a| matches!(a.changes.get(&28), Some(Changes::Ready { .. })));
    for name in ["firefox", "linux", "nodejs", "gimp", "vlc"] {
        assert!(has(&h, w, name), "{name}");
    }
    assert!(has(&h, w, "24.9.0 → 22.20.0"));
    // The running one can't be returned to: it's where the system is.
    assert!(!has(&h, w, "Return…"));
    h.send(Msg::ToggleGeneration(28));
    assert!(!has(&h, w, "vlc"));
}

#[test]
fn pages_scroll() {
    let (mut h, w, _) = sample(WORKS);
    h.send(Msg::Show(Page::Installed));
    let before = h.find(w, "ADDED WITH YUKIMI").expect("the first group");
    h.scroll(w, Point::new(600.0, 400.0), Vec2::new(0.0, 300.0));
    let after = h.find(w, "ADDED WITH YUKIMI");
    assert!(after.is_none_or(|r| r.y0 < before.y0 - 200.0), "{before:?} {after:?}");
    // Each page keeps its own place.
    h.send(Msg::Show(Page::Storage));
    assert!(h.find(w, "The Nix store").is_some_and(|r| r.y0 < 120.0));
}

#[test]
fn nothing_is_changed_when_looking_at_another_system() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let seen = ran.clone();
    let dir = std::env::temp_dir().join(format!("aizome-yukimi-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("nixos")).unwrap();
    std::fs::write(dir.join("nixos/configuration.nix"), "{ pkgs, ... }: { environment.systemPackages = [ pkgs.git ]; }\n")
        .unwrap();
    let paths = aizome_yukimi::paths::Paths { etc: dir.clone(), elsewhere: true };
    let runner = Runner::Mock(Arc::new(move |op: &Operation| {
        seen.lock().unwrap().push(op.clone());
        Vec::new()
    }));
    let mut h = Harness::new(Yukimi::with(Source::System(paths), runner, Clock::System));
    settle(&mut h, |a| !a.loading);
    assert_eq!(h.program().model.configured.system.len(), 1);
    h.send(Msg::Run(Operation::InstallForMe { attr: "htop".into(), unfree: false }));
    h.send(Msg::InstallForEveryone("htop".into()));
    assert!(ran.lock().unwrap().is_empty());
    assert!(h.program().job.is_none());
    assert!(h.program().notes.iter().any(|(_, n)| n.contains("changes nothing")));
    let _ = std::fs::remove_dir_all(dir);
}

/// Click the last button called `label`: an alert's, over a page that
/// has one of the same name.
fn click_last(h: &mut Harness<Yukimi>, w: WindowKey, label: &str) -> bool {
    let found = h.semantics(w).into_iter().filter(|(s, _)| s.label.as_deref() == Some(label)).map(|(_, r)| r).last();
    match found {
        Some(r) => {
            h.click(w, r.center());
            true
        }
        None => false,
    }
}
