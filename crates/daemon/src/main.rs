use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use litebubbles_rustpush_backend::TwoFactorPrompt;
use litebubbles_storage::AppPaths;
use litebubblesd::{run_daemon, run_production_listen, run_production_send, run_production_setup};

fn print_help() {
    println!("litebubblesd — LiteBubbles background service");
    println!();
    println!("Usage:");
    println!("    litebubblesd [DATABASE]");
    println!("    litebubblesd setup [--account ID] [--hardware-file PATH]");
    println!("    litebubblesd send --account ID --to HANDLE [--text TEXT]");
    println!("    litebubblesd listen --account ID [--timeout SECONDS]");
    println!();
    println!("The setup command uses the user-local production compatibility component");
    println!("and the genuine Mac Hardware Info payload. It never imports OpenBubbles");
    println!("user data.");
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

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print_help();
        return Ok(());
    }

    if let Some(command) = arguments.first().map(String::as_str) {
        match command {
            "setup" => return run_setup_command(&arguments[1..]),
            "send" => return run_send_command(&arguments[1..]),
            "listen" => return run_listen_command(&arguments[1..]),
            _ => {}
        }
    }

    let mut database = None;
    for argument in arguments {
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

fn run_setup_command(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let account = option_value(arguments, "--account")?
        .unwrap_or_else(|| prompt_line("Apple Account: ").unwrap_or_default());
    if account.trim().is_empty() {
        return Err("an Apple Account is required".into());
    }
    let hardware_payload = if let Some(path) = option_value(arguments, "--hardware-file")? {
        fs::read_to_string(path).map_err(|_| "could not read the hardware payload file")?
    } else {
        rpassword::prompt_password("Paste the base64 Mac Hardware Info payload: ")?
    };
    let password = rpassword::prompt_password("Apple Account password: ")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_production_setup(
        account,
        hardware_payload,
        password,
        |prompt| match prompt {
            TwoFactorPrompt::TrustedDevice => {
                rpassword::prompt_password("Apple two-factor code: ").unwrap_or_default()
            }
            TwoFactorPrompt::Sms { last_two_digits } => rpassword::prompt_password(format!(
                "Apple SMS code (phone ending in {last_two_digits}): "
            ))
            .unwrap_or_default(),
        },
    ));
    result?;
    println!("Apple setup completed. The daemon can now use this local session.");
    Ok(())
}

fn run_send_command(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let account = required_option(arguments, "--account")?;
    let destination = required_option(arguments, "--to")?;
    let text = option_value(arguments, "--text")?
        .unwrap_or_else(|| prompt_line("Message text: ").unwrap_or_default());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_production_send(account, destination, text))?;
    println!("Message submitted to Apple.");
    Ok(())
}

fn run_listen_command(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let account = required_option(arguments, "--account")?;
    let timeout = option_value(arguments, "--timeout")?
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(300);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_production_listen(account, Duration::from_secs(timeout)))?;
    println!("An incoming Apple message was received.");
    Ok(())
}

fn option_value(
    arguments: &[String],
    option: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let mut value = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == option {
            let next = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{option} requires a value"))?;
            if next.starts_with('-') {
                return Err(format!("{option} requires a value").into());
            }
            if value.replace(next.clone()).is_some() {
                return Err(format!("{option} was supplied more than once").into());
            }
            index += 2;
        } else {
            if arguments[index].starts_with('-') {
                return Err(format!("unknown option: {}", arguments[index]).into());
            }
            index += 1;
        }
    }
    Ok(value)
}

fn required_option(
    arguments: &[String],
    option: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    option_value(arguments, option)?.ok_or_else(|| format!("{option} is required").into())
}

fn prompt_line(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}
