use std::path::PathBuf;

use litebubbles_storage::AppPaths;
use litebubblesd::run_daemon;

fn print_help() {
    println!("litebubblesd — LiteBubbles background service");
    println!();
    println!("Usage: litebubblesd [OPTIONS] [DATABASE]");
    println!();
    println!("Options:");
    println!("    --help       Print this help message");
    println!();
    println!(
        "DATABASE defaults to $XDG_DATA_HOME/litebubbles/litebubbles.sqlite3 (or $HOME/.local/share/litebubbles/litebubbles.sqlite3) and is owned by the daemon."
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .compact()
        .init();

    let mut database = None;
    for argument in std::env::args().skip(1) {
        if argument == "--help" || argument == "-h" {
            print_help();
            return Ok(());
        }
        if argument.starts_with('-') {
            eprintln!("unknown option: {argument}");
            print_help();
            return Ok(());
        }
        if database.replace(PathBuf::from(argument)).is_some() {
            eprintln!("only one database path may be supplied");
            return Ok(());
        }
    }

    let database = match database {
        Some(path) => path,
        None => {
            let paths = AppPaths::from_environment()?;
            paths.ensure_data_dir()?;
            paths.database_path()
        }
    };
    futures_lite::future::block_on(run_daemon(database))
}
