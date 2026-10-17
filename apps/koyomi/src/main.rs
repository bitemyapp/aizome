//! `aizome-koyomi`: the calendar, its alert agent (`--agent`) and its
//! answers for the shell (`--upcoming --json`).

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
}
