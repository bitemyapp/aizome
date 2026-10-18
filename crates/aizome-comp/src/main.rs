//! Aizome's compositor.

mod actions;
mod anim;
mod backend;
mod bindings;
mod cli;
mod cmdkey;
mod config;
mod cursor;
mod decor;
mod focus;
mod grabs;
mod handlers;
mod icons;
mod idle;
mod input;
mod ipc;
mod memory;
mod outputs;
mod night;
mod overlays;
mod popups;
mod protocols;
mod render;
mod requests;
mod screenshot;
mod session;
mod shell;
mod state;
mod stats;
mod svg;
mod text;
mod wm;

use std::process::ExitCode;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::channel;
use smithay::reexports::wayland_server::Display;
use tracing::{error, info, warn};

use crate::backend::Backend;
use crate::cli::BackendKind;
use crate::state::State;

fn main() -> ExitCode {
    let filter = std::env::var("AIZOME_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "info,smithay=warn".into());
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new(filter)).with_writer(std::io::stderr).init();

    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("aizome: {e}");
            }
            eprint!("{}", cli::USAGE);
            return if e.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(2) };
        }
    };
    let config_path = opts.config.clone().unwrap_or_else(aizome_config::Settings::path);
    let settings = aizome_config::Settings::load_from(&config_path).unwrap_or_else(|e| {
        warn!("{}: {e}; using the defaults", config_path.display());
        aizome_config::Settings::default()
    });

    let mut event_loop: EventLoop<'static, State> = match EventLoop::try_new() {
        Ok(l) => l,
        Err(e) => {
            error!("event loop: {e}");
            return ExitCode::FAILURE;
        }
    };
    let display: Display<State> = match Display::new() {
        Ok(d) => d,
        Err(e) => {
            error!("Wayland display: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The command line or AIZOME_RENDERER, else the settings' choice.
    let mut opts = opts;
    if opts.renderer.is_none() {
        opts.renderer = match settings.renderer_choice() {
            aizome_config::RendererChoice::Auto => None,
            aizome_config::RendererChoice::Gpu => Some(cli::RendererKind::Gles),
            aizome_config::RendererChoice::Software => Some(cli::RendererKind::Pixman),
        };
    }
    let backend = match opts.backend {
        BackendKind::Headless => Backend::Headless(backend::headless::Headless::new(opts.renderer, opts.headless_outputs.clone())),
        BackendKind::Winit => match backend::winit::WinitBackend::new(&event_loop) {
            Ok(w) => Backend::Winit(w),
            Err(e) => {
                error!("winit: {e}");
                return ExitCode::FAILURE;
            }
        },
        BackendKind::Udev => match backend::udev::Udev::new(&event_loop, opts.renderer) {
            Ok(u) => Backend::Udev(Box::new(u)),
            Err(e) => {
                error!("udev: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    let mut state = State::new(display, event_loop.handle(), event_loop.get_signal(), backend, opts.clone(), settings);
    match opts.backend {
        BackendKind::Headless => backend::headless::Headless::init(&mut state),
        BackendKind::Winit => backend::winit::WinitBackend::init(&mut state),
        BackendKind::Udev => backend::udev::Udev::init(&mut state),
    }
    info!("{} backend, {} renderer", state.backend.name(), state.backend.renderer_name());

    if let Err(e) = ipc::IpcServer::listen(&mut state) {
        error!("IPC socket: {e}");
        return ExitCode::FAILURE;
    }

    // Settings changes apply live.
    let (tx, rx) = channel::channel::<()>();
    let _watcher = aizome_config::Watcher::spawn_for(config_path, std::time::Duration::from_secs(1), move || {
        let _ = tx.send(());
    });
    let _ = event_loop.handle().insert_source(rx, |event, _, state: &mut State| {
        if let channel::Event::Msg(()) = event
            && let Err(e) = state.reload_config() {
                warn!("settings: {e}");
            }
    });

    state.apply_output_settings();
    state.place_pointer_on_primary();
    state.update_night_light(true);
    idle::start(&mut state);
    session::start(&mut state);

    if let Err(e) = event_loop.run(None, &mut state, |state| state.on_loop_iteration()) {
        error!("event loop: {e}");
    }
    session::stop_children(&mut state.az);
    info!("bye");
    ExitCode::SUCCESS
}
