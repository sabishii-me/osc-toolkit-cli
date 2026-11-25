use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use osc_config::load_and_validate;
use osc_recording::{read_all, stream_ids};
use osc_runtime::{RecordingController, RecordingStatus, ReplayTopology, Runtime};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
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
        /// Restore the recording's routing from a configuration: each recorded stream is the
        /// input it was recorded from, and is sent through that input's mappings to its
        /// outputs. Input sockets are not bound.
        #[arg(
            short,
            long,
            conflicts_with = "target",
            required_unless_present = "target"
        )]
        config: Option<PathBuf>,
        /// Send the selected stream(s) to these destination(s), each datagram to every target.
        /// More than one selected stream also needs `--merge-streams`.
        #[arg(short, long)]
        target: Vec<SocketAddr>,
        /// Replay only this recorded stream (repeatable). Default: every stream.
        #[arg(long)]
        stream: Vec<String>,
        /// Allow `--target` to receive several streams interleaved on the same address.
        #[arg(long, default_value_t = false, requires = "target")]
        merge_streams: bool,
        /// Repeat the recording until stopped, instead of ending after one pass.
        #[arg(long, default_value_t = false)]
        r#loop: bool,
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
    #[error("{0}")]
    Replay(String),
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
        Command::Replay {
            file,
            config,
            target,
            stream,
            merge_streams,
            r#loop,
        } => {
            let destination = match config {
                Some(config) => ReplayDestination::Config(config),
                None => ReplayDestination::Targets {
                    targets: target,
                    merge_streams,
                },
            };
            replay(&file, &destination, &stream, r#loop).await?;
        }
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
    let (command, argument) = line.split_once(' ').unwrap_or((line, ""));
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

/// Where a replay sends its streams.
enum ReplayDestination {
    /// Restore the mappings of this configuration file.
    Config(PathBuf),
    /// Send the selected streams to every target.
    Targets {
        targets: Vec<SocketAddr>,
        merge_streams: bool,
    },
}

/// Replays a recording with its recorded timing (see `osc_runtime::replay`).
///
/// Streams are never merged silently: replaying more than one stream to bare `--target`
/// addresses is refused unless `--merge-streams` is given, because each stream was
/// recorded from its own input and receivers usually tell them apart by port.
async fn replay(
    file: &std::path::Path,
    destination: &ReplayDestination,
    selection: &[String],
    repeat: bool,
) -> Result<(), CliError> {
    let (metadata, records) = read_all(file)?;
    let recorded = stream_ids(&records);
    if let Some(unknown) = selection
        .iter()
        .find(|stream| !recorded.contains(&stream.as_str()))
    {
        return Err(CliError::Replay(format!(
            "--stream `{unknown}` is not in the recording (streams: {})",
            recorded.join(", ")
        )));
    }
    let selected: Vec<&str> = if selection.is_empty() {
        recorded.clone()
    } else {
        recorded
            .iter()
            .copied()
            .filter(|stream| selection.iter().any(|wanted| wanted == stream))
            .collect()
    };

    let topology = match destination {
        ReplayDestination::Config(path) => {
            ReplayTopology::from_config(&load_and_validate(path)?).await?
        }
        ReplayDestination::Targets {
            targets,
            merge_streams,
        } => {
            if selected.len() > 1 && !merge_streams {
                return Err(CliError::Replay(format!(
                    "the recording has {} streams ({}); sending them all to --target would merge \
                     them. Use --config to restore each stream's routing, --stream to pick one, \
                     or --merge-streams to merge on purpose",
                    selected.len(),
                    selected.join(", ")
                )));
            }
            ReplayTopology::fan_out(targets).await?
        }
    };

    println!(
        "Replaying {} from {} (recording start Unix ns: {})",
        if selection.is_empty() {
            "all streams".to_owned()
        } else {
            format!("streams {}", selected.join(", "))
        },
        file.display(),
        metadata.start_unix_ns
    );
    let mut routed = 0;
    for stream in &selected {
        let targets = topology.targets(stream);
        if targets.is_empty() {
            println!("  {stream}: skipped (no mapping)");
        } else {
            routed += 1;
            let targets: Vec<String> = targets.iter().map(ToString::to_string).collect();
            println!("  {stream} -> {}", targets.join(", "));
        }
    }
    if routed == 0 {
        return Err(CliError::Replay(
            "no selected stream has a destination".to_owned(),
        ));
    }

    let selection = (!selection.is_empty()).then_some(selection);
    let sent = osc_runtime::replay(&records, selection, &topology, repeat).await?;
    println!("Replay complete: {sent} datagrams sent");
    Ok(())
}
