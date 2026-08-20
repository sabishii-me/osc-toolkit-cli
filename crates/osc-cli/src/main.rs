use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use osc_config::load_and_validate;

#[derive(Debug, Parser)]
#[command(
    name = "osc-toolkit",
    version,
    about = "High-throughput UDP and OSC routing, recording, and replay"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a configuration file without starting any sockets.
    Validate {
        /// Path to the TOML configuration file.
        #[arg(short, long)]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), osc_config::ConfigError> {
    match cli.command {
        Command::Validate { config } => {
            let validated = load_and_validate(&config)?;
            println!(
                "Configuration is valid: {} inputs, {} outputs, {} mappings",
                validated.inputs.len(),
                validated.outputs.len(),
                validated.mappings.len()
            );
            Ok(())
        }
    }
}
