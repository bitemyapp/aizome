//! Frame times under load, for docs/performance.md.
//!
//! Starts a headless compositor (the `aizome` binary next to this
//! example), opens sidestep-ui windows (examples/sidestep_client) and runs
//! scenarios, reading the compositor's own per-frame CPU (and GPU) times
//! over IPC:
//!
//! ```sh
//! cargo build --release -p aizome-comp --bin aizome --examples
//! target/release/examples/frame_bench --renderer gles --output 2880x1800@2
//! ```

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use aizome_ipc::{Client, DeskSel, Request, Response, Stats};

struct Kill(Vec<Child>);

impl Drop for Kill {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn main() {
    let mut renderer = "gles".to_string();
    let mut output = "2880x1800@2".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--renderer" => renderer = args.next().unwrap(),
            "--output" => output = args.next().unwrap(),
            _ => panic!("usage: frame_bench [--renderer gles|pixman] [--output WxH@S]"),
        }
    }
    let exe = std::env::current_exe().unwrap();
    let examples = exe.parent().unwrap().to_path_buf();
    let target = examples.parent().unwrap().to_path_buf();
    let aizome = target.join("aizome");
    let client_bin = examples.join("sidestep_client");
    let dir = PathBuf::from(format!("/tmp/aizome-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/aizome")).unwrap();
    std::fs::write(dir.join("config/aizome/settings.toml"), "").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = format!("azbench-{}", std::process::id());
    let comp = Command::new(&aizome)
        .args([
            "--backend",
            "headless",
            "--socket",
            &socket,
            "--no-session",
            "--no-shell",
            "--no-xwayland",
            "--renderer",
            &renderer,
            "--headless-output",
            &output,
        ])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("XDG_RUNTIME_DIR", &dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("AIZOME_LOG", "warn").envs(std::env::vars().filter(|(k, _)| k.starts_with("AIZOME_X_")))
        .stdout(Stdio::null())
        .stderr(if std::env::var_os("AIZOME_TRACE").is_some() { Stdio::inherit() } else { Stdio::null() })
        .spawn()
        .expect("start aizome");
    let mut kill = Kill(vec![comp]);
    let ipc = dir.join(format!("aizome.{socket}.sock"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while Client::connect_to(&ipc).is_err() {
        assert!(Instant::now() < deadline, "the compositor didn't start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let req = |r: Request| -> Response { Client::connect_to(&ipc).unwrap().request(&r).unwrap() };
    let spawn = |mode: &str| -> Child {
        Command::new(&client_bin)
            .arg(mode)
            .env("WAYLAND_DISPLAY", &socket)
            .env("XDG_RUNTIME_DIR", &dir)
            .env_remove("DISPLAY")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the client")
    };
    let windows = |n: usize| {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Response::Windows { windows } = req(Request::Windows)
                && windows.len() >= n
            {
                return windows;
            }
            assert!(Instant::now() < deadline, "windows didn't open");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let stats = |reset: bool| -> Stats {
        match req(Request::Stats { reset }) {
            Response::Stats { stats } => stats,
            r => panic!("{r:?}"),
        }
    };
    let report = |name: &str, s: &Stats| {
        let o = &s.outputs[0];
        let ms = |us: f64| us / 1000.0;
        let gpu = o.gpu_us.map(|g| format!("  gpu p50 {:.3} p99 {:.3}", ms(g.p50), ms(g.p99))).unwrap_or_default();
        println!(
            "{name:<28} frames {:>4} skipped {:>3}  cpu p50 {:.3} p95 {:.3} p99 {:.3} max {:.3} mean {:.3} ms{gpu}",
            o.frames_rendered,
            o.frames_skipped,
            ms(o.cpu_us.p50),
            ms(o.cpu_us.p95),
            ms(o.cpu_us.p99),
            ms(o.cpu_us.max),
            ms(o.cpu_us.mean)
        );
    };
    println!("renderer {renderer}, output {output}");

    // Three ordinary windows.
    for _ in 0..3 {
        kill.0.push(spawn("toplevel"));
    }
    let ws = windows(3);
    std::thread::sleep(Duration::from_millis(1500));

    // Idle: nothing changes, nothing is drawn.
    stats(true);
    std::thread::sleep(Duration::from_secs(2));
    report("idle (2s)", &stats(true));

    // Moving a window: composition only.
    let id = ws[0].id;
    let (x0, y0) = (ws[0].geometry.x, ws[0].geometry.y);
    stats(true);
    for i in 0..240 {
        let dx = ((i as f64 / 240.0) * std::f64::consts::TAU).sin() * 300.0;
        req(Request::Move { id: Some(id), x: x0 + dx as i32, y: y0 + (i % 60) });
        std::thread::sleep(Duration::from_millis(16));
    }
    report("move a window (240 steps)", &stats(true));

    // A client drawing every frame.
    kill.0.push(spawn("animate"));
    windows(4);
    std::thread::sleep(Duration::from_millis(500));
    stats(true);
    std::thread::sleep(Duration::from_secs(3));
    report("animating client (3s)", &stats(true));
    // Stop it before the next scenarios.
    if let Some(mut c) = kill.0.pop() {
        let _ = c.kill();
        let _ = c.wait();
    }
    std::thread::sleep(Duration::from_millis(500));

    // Switching Desks: full-output damage while sliding.
    stats(true);
    for i in 0..12 {
        req(Request::SetDesk { desk: DeskSel::Index(if i % 2 == 0 { 2 } else { 1 }) });
        std::thread::sleep(Duration::from_millis(200));
    }
    report("desk switches (12, sliding)", &stats(true));

    // The overview: thumbnails of every window, full damage.
    stats(true);
    for _ in 0..8 {
        req(Request::Overview { state: Some(true) });
        std::thread::sleep(Duration::from_millis(150));
        req(Request::Overview { state: Some(false) });
        std::thread::sleep(Duration::from_millis(150));
    }
    report("overview open/close (8)", &stats(true));

    // Many windows, one moving.
    for _ in 0..7 {
        kill.0.push(spawn("toplevel"));
    }
    windows(10);
    std::thread::sleep(Duration::from_millis(1500));
    req(Request::Focus { id });
    std::thread::sleep(Duration::from_millis(200));
    stats(true);
    for i in 0..120 {
        req(Request::Move { id: Some(id), x: x0 + (i * 3) % 400, y: y0 });
        std::thread::sleep(Duration::from_millis(16));
    }
    report("10 windows, one moving", &stats(true));

    // A full-screen client drawing every frame: the whole output damaged.
    kill.0.push(spawn("animate"));
    let ws = windows(11);
    let anim = ws.iter().find(|w| w.title == "Animating").map(|w| w.id).unwrap();
    req(Request::Fullscreen { id: Some(anim), state: Some(true) });
    std::thread::sleep(Duration::from_millis(800));
    stats(true);
    std::thread::sleep(Duration::from_secs(3));
    report("full-screen client (3s)", &stats(true));

    drop(kill);
    let _ = std::fs::remove_dir_all(&dir);
}
