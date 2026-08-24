use std::{net::SocketAddr, path::PathBuf, process::ExitCode, time::Duration};

use clap::{Parser, Subcommand};
use osc_config::load_and_validate;
use osc_recording::{read_all, replay_delay};
use osc_runtime::Runtime;
use thiserror::Error;
use tokio::{net::UdpSocket, time::sleep};

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
        /// Create a timestamped input-tap recording at this path.
        #[arg(long)]
        record: Option<PathBuf>,
    },
    /// Replay a recording to one UDP target using recorded timing.
    Replay {
        /// Recording file to replay.
        #[arg(short, long)]
        file: PathBuf,
        /// Numeric IPv4 or bracketed IPv6 UDP destination.
        #[arg(short, long)]
        target: SocketAddr,
    },
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Config(#[from] osc_config::ConfigError),
    #[error(transparent)]
    Runtime(#[from] osc_runtime::RuntimeError),
    #[error(transparent)]
    Recording(#[from] osc_recording::RecordingError),
    #[error("replay UDP operation failed: {0}")]
    ReplayIo(std::io::Error),
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
        Command::Run { config, record } => {
            let validated = load_and_validate(&config)?;
            let input_count = validated.inputs.len();
            let output_count = validated.outputs.len();
            let mapping_count = validated.mappings.len();
            let runtime = Runtime::prepare_with_recording(validated, record.as_ref()).await?;
            if let Some(path) = &record {
                println!("Recording input datagrams to {}", path.display());
            }
            println!(
                "Routing started: {input_count} inputs, {output_count} outputs, {mapping_count} mappings"
            );
            runtime.run_until(tokio::signal::ctrl_c()).await?;
            println!("Routing stopped");
        }
        Command::Replay { file, target } => {
            replay(&file, target).await?;
        }
    }
    Ok(())
}

async fn replay(file: &std::path::Path, target: SocketAddr) -> Result<(), CliError> {
    let (metadata, records) = read_all(file)?;
    let bind = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind).await.map_err(CliError::ReplayIo)?;
    println!(
        "Replaying {} datagrams from {} to {} (recording start Unix ns: {})",
        records.len(),
        file.display(),
        target,
        metadata.start_unix_ns
    );
    let mut previous = records.first().map_or(0, |record| record.offset_ns);
    for record in &records {
        let delay = replay_delay(previous, record.offset_ns);
        if delay > Duration::ZERO {
            sleep(delay).await;
        }
        socket
            .send_to(&record.payload, target)
            .await
            .map_err(CliError::ReplayIo)?;
        previous = record.offset_ns;
    }
    println!("Replay complete: {} datagrams", records.len());
    Ok(())
}
