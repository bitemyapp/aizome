//! `aizome-greeter`: Aizome's login screen. See `docs/greeter.md`.

use std::path::PathBuf;

use aizome_greeter::app::Greeter;
use aizome_greeter::compositor::Compositor;
use aizome_greeter::config::{CONFIG, Config};

fn main() {
    let mut config = PathBuf::from(CONFIG);
    let (mut nested, mut demo) = (false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => match args.next() {
                Some(p) => config = p.into(),
                None => usage(2),
            },
            "--nested" => nested = true,
            "--demo" => demo = true,
            "-h" | "--help" => usage(0),
            "-V" | "--version" => {
                println!("aizome-greeter {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(2),
        }
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // A broken configuration mustn't keep anyone from logging in: say so,
    // and come up with the defaults.
    let cfg = Config::load(&config).unwrap_or_else(|e| {
        log::error!("{e}; using the defaults");
        Config::default()
    });
    let compositor = if nested {
        None
    } else {
        match Compositor::start(&cfg.compositor, cfg.compositor_settings.as_ref()) {
            Ok(c) => {
                // Safety: no other thread is running yet.
                unsafe { std::env::set_var("WAYLAND_DISPLAY", &c.socket) };
                Some(c)
            }
            Err(e) => {
                log::error!("the compositor: {e}");
                std::process::exit(1);
            }
        }
    };
    let code = match aizome_kit::try_run(Greeter::new(cfg, true, demo)) {
        Ok(_) => 0,
        Err(e) => {
            log::error!("{e}");
            1
        }
    };
    // The display is free before greetd starts the session.
    drop(compositor);
    std::process::exit(code);
}

fn usage(code: i32) -> ! {
    eprint!("{}", aizome_greeter::USAGE);
    std::process::exit(code);
}
