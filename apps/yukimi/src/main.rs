//! Yukimi 雪見 (Software): Sekka's Nix software manager, made native.
//!
//! `aizome-yukimi [--page overview|installed|discover|updates|history|storage]
//! [--sample]`: open Yukimi (on that page). A second launch hands its
//! arguments to the running Yukimi. `--sample` shows a sample system and
//! changes nothing.
use aizome_yukimi::app::Yukimi;
use aizome_yukimi::instance::{self, Instance};

const HELP: &str = "Usage: aizome-yukimi [--page PAGE] [--sample]

Yukimi (雪見), Aizome's software manager for Sekka's NixOS: see what is
installed and why, find what is available, install it for yourself or for
everyone, update, go back to an earlier version and keep the store tidy.

  --page PAGE   overview, installed, discover, updates, history or storage
  --sample      show a sample system instead of this one (changes nothing)
  -h, --help    show this

Environment:
  AIZOME_YUKIMI_ETC   read the system's configuration from this directory
                      instead of /etc (looking only: nothing can be changed)
  YUKIMI_STALLS       say on standard error when the window is busy for
                      longer than this many milliseconds (50 when empty)
";

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return;
    }
    let page = match aizome_yukimi::app::page_from_args(&args) {
        Ok(page) => page,
        Err(e) => {
            eprintln!("aizome-yukimi: {e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    let sample = args.iter().any(|a| a == "--sample");
    let listener = if sample {
        None
    } else {
        match instance::claim(&args) {
            Instance::Handed => return,
            Instance::First(listener) => listener,
        }
    };
    let mut app = if sample { Yukimi::sample() } else { Yukimi::new() };
    if let Some(page) = page {
        app.page = page;
    }
    app.listener = listener;
    aizome_kit::run(app);
    if !sample {
        instance::release();
    }
}
