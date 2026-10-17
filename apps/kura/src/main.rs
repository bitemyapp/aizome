//! `aizome-kura`: the window, or with a subcommand, the command line
//! (`scan`, `report`, `hints`, `trash`), which needs no display.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if kura::cli::is_cli(&args) {
        std::process::exit(kura::cli::run(&args));
    }
    #[cfg(feature = "gui")]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    #[cfg(feature = "gui")]
    kura::ui::run(args.into_iter().map(std::path::PathBuf::from).collect());
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("aizome-kura was built without its window; try `aizome-kura --help`");
        std::process::exit(2);
    }
}
