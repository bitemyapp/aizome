//! Reloading the compositor's logic under running clients: a test
//! client, foot and (where installed) Electron stay connected while the
//! logic is swapped for a variant built from the same sources (the focus
//! colour drawn pure red), a panic in the variant rolls it back, and a
//! session that starts without any logic shows its windows and takes a
//! logic in later.
//!
//! The variant is built here, in a target directory of its own
//! (`target/reload-test`, kept between runs), with the compositor it is
//! loaded into, so both have the same core whatever `cargo test` was asked
//! to build.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aizome_ipc::{Request, Response, Stats, WindowInfo};
use common::*;

struct Build {
    aizome: PathBuf,
    base: PathBuf,
    red: PathBuf,
}

/// The compositor and two logic libraries, as built from this checkout:
/// as it is, and with `AIZOME_LOGIC_VARIANT=red`.
fn build() -> &'static Build {
    static BUILD: OnceLock<Build> = OnceLock::new();
    BUILD.get_or_init(|| {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base_target =
            std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| workspace.join("target"));
        let target = base_target.join("reload-test");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_string());
        let run = |variant: Option<&str>| {
            let mut cmd = Command::new(&cargo);
            cmd.args(["build", "-p", "aizome-comp", "--manifest-path"])
                .arg(workspace.join("Cargo.toml"))
                .env("CARGO_TARGET_DIR", &target)
                .env_remove("AIZOME_LOGIC_VARIANT");
            if let Some(v) = variant {
                cmd.env("AIZOME_LOGIC_VARIANT", v);
            }
            let status = cmd.status().expect("run cargo");
            assert!(status.success(), "building the compositor (variant {variant:?}) failed");
        };
        let logic = target.join("debug/libaizome_comp_logic.so");
        run(None);
        let base = target.join("logic-base.so");
        std::fs::copy(&logic, &base).unwrap();
        run(Some("red"));
        let red = target.join("logic-red.so");
        std::fs::copy(&logic, &red).unwrap();
        // Put the plain one back for the next run's first build.
        run(None);
        Build { aizome: target.join("debug/aizome"), base, red }
    })
}

fn stats(c: &Compositor) -> Stats {
    match c.request(Request::Stats { reset: false }) {
        Response::Stats { stats } => stats,
        r => panic!("{r:?}"),
    }
}

/// What must survive a reload: every window's place, group, Desk and
/// state, and which has focus.
fn desktop(ws: &[WindowInfo]) -> Vec<String> {
    let mut v: Vec<String> = ws
        .iter()
        .map(|w| {
            format!(
                "{} {} desk={:?} group={:?} members={:?} active={} focused={} min={} hidden={} fill={} {:?}",
                w.id,
                w.app_id,
                w.desk,
                w.group,
                w.group_members,
                w.group_active,
                w.focused,
                w.minimised,
                w.hidden,
                w.fill,
                (w.geometry.x, w.geometry.y, w.geometry.w, w.geometry.h)
            )
        })
        .collect();
    v.sort();
    v
}

/// Pixels of exactly the variant's red.
fn red_pixels(img: &Image) -> usize {
    img.rgba.chunks(4).filter(|p| p[0] >= 0xfa && p[1] <= 0x05 && p[2] <= 0x05).count()
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Proc {
    fn running(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

/// A program as a client of the test compositor.
fn spawn(c: &Compositor, argv: &[&str]) -> Proc {
    let child = Command::new(argv[0])
        .args(&argv[1..])
        .env_remove("DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env("WAYLAND_DISPLAY", &c.socket)
        .env("XDG_RUNTIME_DIR", &c.dir)
        .env("HOME", c.dir.join("home"))
        .env("XDG_CONFIG_HOME", c.dir.join("home/.config"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            std::fs::File::create(
                c.dir.join(format!("{}.log", Path::new(argv[0]).file_name().unwrap().to_string_lossy())),
            )
            .unwrap(),
        )
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", argv[0]));
    Proc(child)
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

const ELECTRON: &str = "/nix/store/nwn5gamww3fr51mwbrg7a0l9azrl3c9n-electron-44.5.1/bin/electron";

/// A small Electron app: one 400 × 300 window.
fn electron(c: &Compositor) -> Option<Proc> {
    let bin = std::env::var("AIZOME_TEST_ELECTRON").ok().map(PathBuf::from).unwrap_or_else(|| ELECTRON.into());
    if !bin.is_file() {
        eprintln!("no Electron at {}; testing without it", bin.display());
        return None;
    }
    let app = c.dir.join("electron-app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("package.json"), r#"{"name":"aizome-reload-test","main":"main.js"}"#).unwrap();
    std::fs::write(
        app.join("main.js"),
        r#"const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => {
  const w = new BrowserWindow({ width: 400, height: 300, title: 'Electron under reload' });
  w.loadURL('data:text/html,<body style="background:%23884488">Electron</body>');
});"#,
    )
    .unwrap();
    let bin = bin.to_string_lossy().into_owned();
    let app = app.to_string_lossy().into_owned();
    Some(spawn(
        c,
        &[
            &bin,
            "--ozone-platform=wayland",
            "--enable-features=UseOzonePlatform",
            "--no-sandbox",
            "--disable-gpu",
            &app,
        ],
    ))
}

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn reloading_the_logic_keeps_every_client_and_the_desktop() {
    let b = build();
    let c = Compositor::start(Options {
        exe: Some(&b.aizome),
        logic: Some(&b.base),
        settings: "",
        outputs: &["1600x1000@1"],
        ..Default::default()
    });
    let base_build = stats(&c).logic.build;
    assert!(!base_build.is_empty(), "a logic is loaded:\n{}", c.log_text());

    // A test client in three windows, foot, and Electron.
    let a = c.client(Spec::new("org.example.a", 0x2266cc).size(420, 300));
    let a2 = c.client(Spec::new("org.example.a2", 0x22cc66).size(420, 300));
    let d = c.client(Spec::new("org.example.d", 0xcc6622).size(300, 200));
    let wa = c.wait_window("org.example.a");
    let wa2 = c.wait_window("org.example.a2");
    let wd = c.wait_window("org.example.d");
    let foot_bin = which("foot");
    let mut foot = foot_bin.as_ref().map(|f| spawn(&c, &[&f.to_string_lossy(), "--app-id", "foot", "sh"]));
    if foot.is_some() {
        c.wait_windows("foot's window", |ws| ws.iter().any(|w| w.app_id == "foot"));
    } else {
        eprintln!("foot isn't installed; testing without it");
    }
    let mut electron = electron(&c);
    let electron_pid = electron.as_ref().map(|e| e.0.id() as i32);
    if let Some(pid) = electron_pid {
        let deadline = Instant::now() + Duration::from_secs(40);
        while !c.windows().iter().any(|w| w.pid == Some(pid)) {
            assert!(Instant::now() < deadline, "Electron's window didn't open:\n{}", c.log_text());
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // A desktop to keep: a tab group, a window on Desk 2, one filled,
    // focus on the group's showing member.
    c.ok(Request::Stack { a: wa.id, b: wa2.id });
    c.ok(Request::MoveToDesk { id: Some(wd.id), desk: aizome_ipc::DeskSel::Index(2), follow: false });
    c.ok(Request::Focus { id: wa2.id });
    c.settle();
    let before = desktop(&c.windows());
    let desks_before = c.request(Request::Desks);
    let shot_a = c.screenshot();
    keep(&shot_a, "reload-before");
    assert_eq!(red_pixels(&shot_a), 0, "the plain logic draws no pure red");

    // Swap in the variant.
    let report = match c.request(Request::Reload {
        logic: true,
        shell: false,
        from: Some(b.red.to_string_lossy().into()),
        check: false,
    }) {
        Response::Reload { report } => report,
        r => panic!("{r:?}"),
    };
    assert_eq!(report.logic.action, "reloaded", "{report:?}\n{}", c.log_text());
    c.settle();
    let s = stats(&c);
    assert_ne!(s.logic.build, base_build, "the variant is another build");
    assert_eq!(s.logic.reloads, 1);
    assert_eq!(desktop(&c.windows()), before, "windows, groups, Desks and focus survive the reload");
    assert_eq!(c.request(Request::Desks), desks_before);
    for (cl, name) in [(&a, "a"), (&a2, "a2"), (&d, "d")] {
        assert!(cl.connected(), "test client {name} is still connected");
    }
    if let Some(f) = &mut foot {
        assert!(f.running(), "foot is still running");
    }
    if let Some(e) = &mut electron {
        assert!(e.running(), "Electron is still running");
    }
    let shot_b = c.screenshot();
    keep(&shot_b, "reload-variant");
    assert!(red_pixels(&shot_b) > 200, "the variant draws the focused tab red: {} red pixels", red_pixels(&shot_b));

    // The variant fails: back to the logic before it, desktop intact.
    match c.request(Request::Action { action: "test-panic".into() }) {
        Response::Error { .. } => {}
        r => panic!("a panicking logic answers with an error, not {r:?}"),
    }
    wait_for("the rollback", 10, || {
        let s = stats(&c);
        s.logic.rollbacks == 1 && s.logic.healthy && s.logic.build == base_build
    });
    let s = stats(&c);
    assert!(s.logic.last_error.as_deref().is_some_and(|e| e.contains("panics on purpose")), "{:?}", s.logic);
    c.settle();
    assert_eq!(desktop(&c.windows()), before, "the rollback keeps the desktop");
    let shot_c = c.screenshot();
    keep(&shot_c, "reload-rolled-back");
    assert_eq!(red_pixels(&shot_c), 0, "the plain logic draws again");
    for (cl, name) in [(&a, "a"), (&a2, "a2"), (&d, "d")] {
        assert!(cl.connected(), "test client {name} survived the rollback");
    }
    if let Some(f) = &mut foot {
        assert!(f.running(), "foot survived the rollback");
    }
    if let Some(e) = &mut electron {
        assert!(e.running(), "Electron survived the rollback");
        assert!(c.windows().iter().any(|w| w.pid == electron_pid), "Electron's window is still there");
    }

    // The windows still take input from the reloaded logic: focus moves.
    c.ok(Request::Focus { id: wd.id });
    let ws = c.windows();
    assert!(ws.iter().find(|w| w.id == wd.id).unwrap().focused);
    assert_eq!(ws.iter().find(|w| w.id == wd.id).unwrap().desk, Some(2));
}

#[test]
fn without_a_logic_windows_still_show_and_a_reload_brings_it() {
    let b = build();
    let missing = Path::new("/nonexistent/libaizome_comp_logic.so");
    let c =
        Compositor::start(Options { exe: Some(&b.aizome), logic: Some(missing), settings: "", ..Default::default() });
    assert!(stats(&c).logic.build.is_empty(), "no logic is loaded");
    let a = c.client(Spec::new("org.example.bare", 0x33aa77).size(300, 200));
    let w = c.wait_window("org.example.bare");
    assert!(w.focused, "the recovery path focuses a new window");
    c.settle();
    let img = c.screenshot();
    let (x, y) = (w.geometry.x + 150, w.geometry.y + 100);
    assert_px(&img, x, y, rgb(0x33aa77), "the window, drawn without a logic");

    match c.request(Request::Reload {
        logic: true,
        shell: false,
        from: Some(b.base.to_string_lossy().into()),
        check: false,
    }) {
        Response::Reload { report } => assert_eq!(report.logic.action, "reloaded", "{report:?}"),
        r => panic!("{r:?}"),
    }
    let ws = c.windows();
    let after = ws.iter().find(|x| x.app_id == "org.example.bare").expect("the logic took the window in");
    assert_eq!((after.id, after.geometry.x, after.geometry.y), (w.id, w.geometry.x, w.geometry.y), "where it was");
    assert!(a.connected());
    c.settle();
    // Now with its tab and frame.
    let img = c.screenshot();
    assert_px(&img, x, y, rgb(0x33aa77), "the window");
    assert!(
        !near(img.px(w.geometry.x + 2, w.geometry.y - 12), img.px(w.geometry.x + 2, w.geometry.y - 60)),
        "a tab above it"
    );
}

#[test]
fn a_library_built_against_another_core_is_refused() {
    let b = build();
    let c = Compositor::start(Options { exe: Some(&b.aizome), logic: Some(&b.base), ..Default::default() });
    // A copy whose ABI mark says another core.
    let mut bytes = std::fs::read(&b.base).unwrap();
    let mark = b"AIZOME-LOGIC-ABI:";
    let at = bytes.windows(mark.len()).position(|w| w == mark).expect("the mark") + mark.len();
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    let other = c.dir.join("other-core.so");
    std::fs::write(&other, &bytes).unwrap();
    let report = match c.request(Request::Reload {
        logic: true,
        shell: false,
        from: Some(other.to_string_lossy().into()),
        check: true,
    }) {
        Response::Reload { report } => report,
        r => panic!("{r:?}"),
    };
    assert_eq!(report.logic.action, "refused", "{report:?}");
    let report = match c.request(Request::Reload {
        logic: true,
        shell: false,
        from: Some(other.to_string_lossy().into()),
        check: false,
    }) {
        Response::Reload { report } => report,
        r => panic!("{r:?}"),
    };
    assert_eq!(report.logic.action, "refused", "{report:?}");
    assert!(report.logic.detail.contains("another compositor core"), "{report:?}");
    assert_eq!(report.core.action, "same", "the installed core is the running one: {report:?}");
    let s = stats(&c);
    assert!(s.logic.healthy, "the running logic stays");
    assert_eq!(s.logic.reloads, 0);
}
