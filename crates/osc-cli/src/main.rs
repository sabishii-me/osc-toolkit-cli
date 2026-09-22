use std::{net::SocketAddr, path::PathBuf, process::ExitCode, time::Duration};

use clap::{Parser, Subcommand};
use osc_config::load_and_validate;
use osc_recording::{read_all, replay_delay};
use osc_runtime::{RecordingController, RecordingStatus, Runtime};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    time::sleep,
};

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
    /// Validate a configuration file without starting sockets.
    Validate {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Keep data sockets ready and expose a local recording control endpoint.
    Serve {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7012")]
        control: SocketAddr,
    },
    /// Control recording on an already running service.
    Record {
        #[arg(long, default_value = "127.0.0.1:7012")]
        control: SocketAddr,
        #[command(subcommand)]
        action: RecordAction,
    },
    /// Replay a finalized recording using recorded timing.
    Replay {
        #[arg(short, long)]
        file: PathBuf,
        /// Destination(s). Pass more than once to send EVERY datagram to EACH target - one
        /// recording fans out to many receivers with the recorded timing preserved.
        #[arg(short, long, required = true)]
        target: Vec<SocketAddr>,
    },
}

#[derive(Debug, Subcommand)]
enum RecordAction {
    /// Start recording immediately to a new file.
    Start {
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Stop immediately and wait until the file is durable.
    Stop,
    /// Report current recording state.
    Status,
    /// Stop the service after finalizing any active recording.
    Shutdown,
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Config(#[from] osc_config::ConfigError),
    #[error(transparent)]
    Runtime(#[from] osc_runtime::RuntimeError),
    #[error(transparent)]
    Recording(#[from] osc_recording::RecordingError),
    #[error("control operation failed: {0}")]
    ControlIo(std::io::Error),
    #[error("control service rejected the command: {0}")]
    ControlRejected(String),
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
        Command::Serve { config, control } => serve(config, control).await?,
        Command::Record { control, action } => {
            let command = match action {
                RecordAction::Start { file } => format!("START {}", file.display()),
                RecordAction::Stop => "STOP".to_owned(),
                RecordAction::Status => "STATUS".to_owned(),
                RecordAction::Shutdown => "SHUTDOWN".to_owned(),
            };
            println!("{}", send_control(control, &command).await?);
        }
        Command::Replay { file, target } => replay(&file, &target).await?,
    }
    Ok(())
}

async fn serve(config: PathBuf, control_address: SocketAddr) -> Result<(), CliError> {
    let validated = load_and_validate(&config)?;
    let input_count = validated.inputs.len();
    let (runtime, recorder) = Runtime::prepare_controlled(validated).await?;
    let listener = TcpListener::bind(control_address)
        .await
        .map_err(CliError::ControlIo)?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let runtime_task = tokio::spawn(runtime.run_until(async {
        let _ = shutdown_rx.await;
        Ok(())
    }));
    println!("READY inputs={input_count} control={control_address}");

    loop {
        let (stream, _) = listener.accept().await.map_err(CliError::ControlIo)?;
        if handle_control(stream, &recorder).await? {
            break;
        }
    }

    let _ = shutdown_tx.send(());
    runtime_task
        .await
        .map_err(osc_runtime::RuntimeError::TaskJoin)??;
    println!("SHUTDOWN complete");
    Ok(())
}

async fn handle_control(
    stream: TcpStream,
    recorder: &RecordingController,
) -> Result<bool, CliError> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let Some(line) = lines.next_line().await.map_err(CliError::ControlIo)? else {
        return Ok(false);
    };
    let line = line.trim();
    let (command, argument) = line.split_once(' ').map_or((line, ""), |parts| parts);
    let (response, shutdown) = match command.to_ascii_uppercase().as_str() {
        "START" if !argument.trim().is_empty() => match recorder.start(argument.trim()) {
            Ok(()) => (format!("RECORDING file={}", argument.trim()), false),
            Err(error) => (format!("ERROR {error}"), false),
        },
        "STOP" => match recorder.stop() {
            Ok((path, records)) => (
                format!("FINALIZED file={} records={records}", path.display()),
                false,
            ),
            Err(error) => (format!("ERROR {error}"), false),
        },
        "STATUS" => match recorder.status()? {
            RecordingStatus::Idle => ("STATUS idle".to_owned(), false),
            RecordingStatus::Recording(path) => {
                (format!("STATUS recording file={}", path.display()), false)
            }
        },
        "SHUTDOWN" => {
            if matches!(recorder.status()?, RecordingStatus::Recording(_)) {
                recorder.stop()?;
            }
            ("SHUTDOWN accepted".to_owned(), true)
        }
        _ => (
            "ERROR expected START <file>, STOP, STATUS, or SHUTDOWN".to_owned(),
            false,
        ),
    };
    writer
        .write_all(format!("{response}\n").as_bytes())
        .await
        .map_err(CliError::ControlIo)?;
    writer.shutdown().await.map_err(CliError::ControlIo)?;
    Ok(shutdown)
}

async fn send_control(address: SocketAddr, command: &str) -> Result<String, CliError> {
    let mut stream = TcpStream::connect(address)
        .await
        .map_err(CliError::ControlIo)?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .await
        .map_err(CliError::ControlIo)?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .await
        .map_err(CliError::ControlIo)?;
    let response = response.trim().to_owned();
    if let Some(reason) = response.strip_prefix("ERROR ") {
        return Err(CliError::ControlRejected(reason.to_owned()));
    }
    Ok(response)
}

async fn replay(file: &std::path::Path, targets: &[SocketAddr]) -> Result<(), CliError> {
    let (metadata, records) = read_all(file)?;
    // One socket serves every target. Bind the family of the FIRST target; a mixed-family target list is
    // rejected rather than silently sending to only part of it.
    let first_v4 = targets[0].is_ipv4();
    if targets.iter().any(|target| target.is_ipv4() != first_v4) {
        return Err(CliError::ReplayIo(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "all --target addresses must use the same address family",
        )));
    }
    let socket = UdpSocket::bind(if first_v4 { "0.0.0.0:0" } else { "[::]:0" })
        .await
        .map_err(CliError::ReplayIo)?;
    let destinations = targets
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "Replaying {} datagrams from {} to {} target(s): {} (recording start Unix ns: {})",
        records.len(),
        file.display(),
        targets.len(),
        destinations,
        metadata.start_unix_ns
    );
    let mut previous = records.first().map_or(0, |record| record.offset_ns);
    for record in &records {
        // The recorded TIMING is preserved once, then the datagram fans out to every target in turn, so all
        // receivers see the same frame at the same moment.
        let delay = replay_delay(previous, record.offset_ns);
        if delay > Duration::ZERO {
            sleep(delay).await;
        }
        for target in targets {
            socket
                .send_to(&record.payload, target)
                .await
                .map_err(CliError::ReplayIo)?;
        }
        previous = record.offset_ns;
    }
    println!(
        "Replay complete: {} datagrams x {} target(s)",
        records.len(),
        targets.len()
    );
    Ok(())
}
