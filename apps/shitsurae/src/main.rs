//! `aizome-shitsurae`: Aizome's Settings.
use shitsurae::app::{Shitsurae, Start};
use shitsurae::instance::{self, Instance};

const HELP: &str = "Usage: aizome-shitsurae [--pane ID] [--sample]
       aizome-shitsurae --list-settings [--json]

Shitsurae (しつらえ), Aizome's Settings: arrange the desktop for its season
and its guests. Changes apply at once.

  --pane ID          open a pane (keyboard), or an option in one
                     (keyboard.caps-lock)
  --list-settings    print every pane and option; with --json, as JSON for
                     the command palette (id, pane, title, path, keywords)
  --sample           show a sample computer instead of this one: nothing it
                     does changes the system (settings still go to
                     $XDG_CONFIG_HOME/aizome/settings.toml)
  -h, --help         show this
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return;
    }
    if args.iter().any(|a| a == "--list-settings") {
        let panes = shitsurae::panes::all();
        if args.iter().any(|a| a == "--json") {
            println!("{}", shitsurae::listing::json(&panes));
        } else {
            print!("{}", shitsurae::listing::table(&panes));
        }
        return;
    }
    env_logger::init();
    let start = Start::from_args(&args);
    let sample = args.iter().any(|a| a == "--sample");
    let listener = if sample {
        None
    } else {
        match instance::claim(&args) {
            Instance::Handed => return,
            Instance::First(listener) => listener,
        }
    };
    let mut app = if sample {
        Shitsurae::sample(aizome_config::Settings::path(), start)
    } else {
        Shitsurae::live(start)
    };
    app.listener = listener;
    aizome_kit::run(app);
    if !sample {
        instance::release();
    }
}
