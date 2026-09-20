use std::path::PathBuf;

use litebubblesd::run_daemon;

fn print_help() {
    println!("litebubblesd — LiteBubbles background service");
    println!();
    println!("Usage: litebubblesd [OPTIONS] [DATABASE]");
    println!();
    println!("Options:");
    println!("    --help       Print this help message");
    println!();
    println!("DATABASE defaults to litebubbles.sqlite3 and is owned by the daemon.");
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

    let database = database.unwrap_or_else(|| PathBuf::from("litebubbles.sqlite3"));
    futures_lite::future::block_on(run_daemon(database))
}
