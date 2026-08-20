use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use osc_config::load_and_validate;
use osc_runtime::Runtime;
use thiserror::Error;

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
    /// Run the configured UDP routing topology until interrupted.
    Run {
        /// Path to the TOML configuration file.
        #[arg(short, long)]
        config: PathBuf,
    },
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Config(#[from] osc_config::ConfigError),
    #[error(transparent)]
    Runtime(#[from] osc_runtime::RuntimeError),
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Validate { config } => {
            let validated = load_and_validate(&config)?;
            println!(
                "Configuration is valid: {} inputs, {} outputs, {} mappings",
                validated.inputs.len(),
                validated.outputs.len(),
                validated.mappings.len()
            );
        }
        Command::Run { config } => {
            let validated = load_and_validate(&config)?;
            let input_count = validated.inputs.len();
            let output_count = validated.outputs.len();
            let mapping_count = validated.mappings.len();
            let runtime = Runtime::prepare(validated).await?;
            println!(
                "Routing started: {input_count} inputs, {output_count} outputs, {mapping_count} mappings"
            );
            runtime.run_until(tokio::signal::ctrl_c()).await?;
            println!("Routing stopped");
        }
    }
    Ok(())
}
