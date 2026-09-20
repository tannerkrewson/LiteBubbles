fn print_help() {
    println!("litebubblesd — LiteBubbles background service");
    println!();
    println!("Usage: litebubblesd [OPTIONS]");
    println!();
    println!("Options:");
    println!("    --help       Print this help message");
}

fn main() {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return;
    }

    println!("litebubblesd is not configured yet; see the project issue backlog.");
}
