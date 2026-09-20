use litebubbles_validation_provider::{
    EXPECTED_COMPONENT_VERSION, default_compat_root, install_official_artifact,
    remove_installed_component,
};
use std::env;
use std::path::PathBuf;

fn usage() {
    println!(
        "Usage:\n  litebubbles-validation-component install ARCHIVE [--data-home DIR]\n  litebubbles-validation-component remove [--data-home DIR]\n  litebubbles-validation-component status [--data-home DIR]"
    );
}

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("validation component setup failed: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        usage();
        return Err("a command is required".to_string());
    };
    if command == "--help" || command == "-h" {
        usage();
        return Ok(());
    }

    let mut data_home = None;
    let mut positional = Vec::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--data-home" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--data-home requires a directory".to_string())?;
                data_home = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with('-') => return Err("unknown option".to_string()),
            value => {
                positional.push(value.to_string());
                index += 1;
            }
        }
    }

    let compat_root = match data_home {
        Some(path) => {
            if !path.is_absolute() {
                return Err("data home must be absolute".to_string());
            }
            path.join("litebubbles/compat")
        }
        None => default_compat_root().map_err(|error| error.to_string())?,
    };

    match command {
        "install" => {
            let archive = positional
                .first()
                .ok_or_else(|| "install requires an official archive path".to_string())?;
            let path = install_official_artifact(archive, &compat_root)
                .map_err(|error| error.to_string())?;
            println!("Installed validation component {EXPECTED_COMPONENT_VERSION}.");
            println!("Component directory: {}", path.display());
            Ok(())
        }
        "remove" | "reset" => {
            let removed =
                remove_installed_component(&compat_root).map_err(|error| error.to_string())?;
            if removed {
                println!("Removed validation component {EXPECTED_COMPONENT_VERSION}.");
            } else {
                println!("No validation component was installed.");
            }
            Ok(())
        }
        "status" => {
            let path = compat_root.join(EXPECTED_COMPONENT_VERSION);
            println!(
                "{}",
                if path.is_dir() {
                    "A validation component directory is present."
                } else {
                    "No validation component is installed."
                }
            );
            Ok(())
        }
        _ => {
            usage();
            Err("unknown command".to_string())
        }
    }
}
