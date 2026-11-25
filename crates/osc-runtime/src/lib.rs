//! Bounded-memory UDP routing runtime.

use std::{
    future::Future,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use osc_config::ValidatedConfig;
use osc_recording::{Record, ReplayCursor, Writer};
use thiserror::Error;
use tokio::{net::UdpSocket, sync::watch, task::JoinSet, time::sleep};

/// Maximum UDP payload accepted by the runtime.
pub const MAX_DATAGRAM_SIZE: usize = 65_507;

/// A runtime startup or forwarding failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// The configuration has no input listeners.
    #[error("runtime requires at least one input")]
    NoInputs,
    /// Binding an input socket failed.
    #[error("failed to bind input `{input}` at {address}: {source}")]
    BindInput {
        /// Input identifier.
        input: String,
        /// Requested local address.
        address: SocketAddr,
        /// Underlying socket error.
        source: io::Error,
    },
    /// Creating an output socket failed.
    #[error("failed to create output `{output}` for {target}: {source}")]
    CreateOutput {
        /// Output identifier.
        output: String,
        /// Remote target address.
        target: SocketAddr,
        /// Underlying socket error.
        source: io::Error,
    },
    /// A recording is already active.
    #[error("a recording is already active at `{0}`")]
    RecordingActive(PathBuf),
    /// No recording is active.
    #[error("no recording is active")]
    RecordingInactive,
    /// Creating or writing the recording failed.
    #[error(transparent)]
    Recording(#[from] osc_recording::RecordingError),
    /// A recording lock was poisoned by a failed task.
    #[error("recording writer lock was poisoned")]
    RecordingLock,
    /// Waiting for the requested shutdown signal failed.
    #[error("failed to wait for shutdown: {0}")]
    Shutdown(io::Error),
    /// A forwarding task failed.
    #[error("input task `{input}` failed: {source}")]
    InputTask {
        /// Input identifier.
        input: String,
        /// Underlying socket error.
        source: io::Error,
    },
    /// Sending a replayed datagram failed.
    #[error("replay of stream `{stream}` to {target} failed: {source}")]
    Replay {
        /// Recorded stream identifier.
        stream: String,
        /// Destination address.
        target: SocketAddr,
        /// Underlying socket error.
        source: io::Error,
    },
    /// A forwarding task terminated unexpectedly.
    #[error("input task terminated unexpectedly: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

/// A prepared runtime whose sockets have all been created successfully.
pub struct Runtime {
    inputs: Vec<PreparedInput>,
    recorder: RecordingController,
}

struct PreparedInput {
    id: String,
    socket: UdpSocket,
    destinations: Arc<[Destination]>,
    recorder: RecordingController,
}

#[derive(Clone)]
pub struct RecordingController {
    state: Arc<Mutex<RecordingState>>,
}

struct RecordingState {
    active: Option<ActiveRecording>,
}
struct ActiveRecording {
    path: PathBuf,
    writer: Writer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingStatus {
    Idle,
    Recording(PathBuf),
}

impl RecordingController {
    /// Starts a recording without rebinding any input sockets.
    ///
    /// # Errors
    /// Returns an error if recording is active or the file cannot be created.
    pub fn start(&self, path: impl AsRef<Path>) -> Result<(), RuntimeError> {
        let path = path.as_ref().to_path_buf();
        let mut state = self.state.lock().map_err(|_| RuntimeError::RecordingLock)?;
        if let Some(active) = &state.active {
            return Err(RuntimeError::RecordingActive(active.path.clone()));
        }
        let writer = Writer::create(&path)?;
        state.active = Some(ActiveRecording { path, writer });
        Ok(())
    }

    /// Stops the active recording and durably synchronizes its file.
    ///
    /// # Errors
    /// Returns an error if recording is idle or finalization fails.
    pub fn stop(&self) -> Result<(PathBuf, u64), RuntimeError> {
        let active = self
            .state
            .lock()
            .map_err(|_| RuntimeError::RecordingLock)?
            .active
            .take()
            .ok_or(RuntimeError::RecordingInactive)?;
        let records = active.writer.finish()?;
        Ok((active.path, records))
    }

    /// Reports whether recording is idle or active.
    ///
    /// # Errors
    /// Returns an error if the recorder state lock is unavailable.
    pub fn status(&self) -> Result<RecordingStatus, RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::RecordingLock)?;
        Ok(state
            .active
            .as_ref()
            .map_or(RecordingStatus::Idle, |active| {
                RecordingStatus::Recording(active.path.clone())
            }))
    }
}

struct Destination {
    socket: Arc<UdpSocket>,
    target: SocketAddr,
}

/// Creates one socket for an output, in the target's address family.
async fn bind_output(id: &str, target: SocketAddr) -> Result<Arc<UdpSocket>, RuntimeError> {
    let bind_address = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket =
        UdpSocket::bind(bind_address)
            .await
            .map_err(|source| RuntimeError::CreateOutput {
                output: id.to_owned(),
                target,
                source,
            })?;
    Ok(Arc::new(socket))
}

/// Compiles the mapping edges into per-input destination lists (indexed like
/// [`ValidatedConfig::inputs`]). Live routing and replay both use this, so a
/// recording replays through exactly the topology that serves live traffic.
async fn compile_destinations(
    config: &ValidatedConfig,
) -> Result<Vec<Vec<Destination>>, RuntimeError> {
    let mut output_sockets = Vec::with_capacity(config.outputs.len());
    for output in &config.outputs {
        output_sockets.push(bind_output(output.id.as_str(), output.target).await?);
    }
    let mut destinations_by_input: Vec<Vec<Destination>> =
        (0..config.inputs.len()).map(|_| Vec::new()).collect();
    for mapping in &config.mappings {
        let output = &config.outputs[mapping.output_index];
        destinations_by_input[mapping.input_index].push(Destination {
            socket: Arc::clone(&output_sockets[mapping.output_index]),
            target: output.target,
        });
    }
    Ok(destinations_by_input)
}

/// Where each recorded stream goes during replay.
///
/// A recording's stream identifiers are the identifiers of the inputs it was
/// recorded from, so [`ReplayTopology::from_config`] restores the original
/// routing: each stream is sent through its input's mapping edges.
/// [`ReplayTopology::fan_out`] instead sends every stream to every target.
pub struct ReplayTopology {
    /// Mapping restoration: input (stream) identifier and its destinations.
    mapped: Vec<(String, Vec<Destination>)>,
    /// Fan-out: destinations shared by every stream.
    every_stream: Vec<Destination>,
}

impl ReplayTopology {
    /// Restores the mapping edges of `config`. Input sockets are not bound:
    /// recorded datagrams take the place of received ones.
    ///
    /// # Errors
    /// Returns an error if an output socket cannot be created.
    pub async fn from_config(config: &ValidatedConfig) -> Result<Self, RuntimeError> {
        let destinations = compile_destinations(config).await?;
        let mapped = config
            .inputs
            .iter()
            .zip(destinations)
            .map(|(input, destinations)| (input.id.to_string(), destinations))
            .collect();
        Ok(Self {
            mapped,
            every_stream: Vec::new(),
        })
    }

    /// Sends every replayed stream to each of `targets`.
    ///
    /// # Errors
    /// Returns an error if a socket cannot be created.
    pub async fn fan_out(targets: &[SocketAddr]) -> Result<Self, RuntimeError> {
        let mut every_stream = Vec::with_capacity(targets.len());
        for target in targets {
            every_stream.push(Destination {
                socket: bind_output(&target.to_string(), *target).await?,
                target: *target,
            });
        }
        Ok(Self {
            mapped: Vec::new(),
            every_stream,
        })
    }

    fn destinations(&self, stream_id: &str) -> &[Destination] {
        if !self.every_stream.is_empty() {
            return &self.every_stream;
        }
        self.mapped
            .iter()
            .find(|(input, _)| input == stream_id)
            .map_or(&[], |(_, destinations)| destinations.as_slice())
    }

    /// Destination addresses of a stream; empty when it is not routed.
    #[must_use]
    pub fn targets(&self, stream_id: &str) -> Vec<SocketAddr> {
        self.destinations(stream_id)
            .iter()
            .map(|destination| destination.target)
            .collect()
    }
}

/// Replays `records` with their recorded timing through `topology`.
///
/// Only the streams in `selection` are replayed when it is given (see
/// [`ReplayCursor`]). Streams without a destination are skipped. With
/// `repeat`, passes continue until the future is dropped. Returns the number
/// of datagrams sent.
///
/// # Errors
/// Returns an error when a send fails.
pub async fn replay(
    records: &[Record],
    selection: Option<&[String]>,
    topology: &ReplayTopology,
    repeat: bool,
) -> Result<u64, RuntimeError> {
    let mut sent = 0_u64;
    loop {
        for (delay, record) in ReplayCursor::new(records, selection) {
            if delay > Duration::ZERO {
                sleep(delay).await;
            }
            for destination in topology.destinations(&record.stream_id) {
                destination
                    .socket
                    .send_to(&record.payload, destination.target)
                    .await
                    .map_err(|source| RuntimeError::Replay {
                        stream: record.stream_id.clone(),
                        target: destination.target,
                        source,
                    })?;
                sent += 1;
            }
        }
        if !repeat {
            return Ok(sent);
        }
    }
}

impl Runtime {
    /// Creates every input and output socket transactionally.
    ///
    /// If any socket cannot be created, all sockets already created by this
    /// function are dropped before the error is returned.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] when there are no inputs or a socket cannot be
    /// created.
    pub async fn prepare(config: ValidatedConfig) -> Result<Self, RuntimeError> {
        let (runtime, _) = Self::prepare_controlled(config).await?;
        Ok(runtime)
    }

    /// Binds all sockets and returns an independently controlled recorder.
    ///
    /// # Errors
    /// Returns an error if no input exists or a socket cannot be prepared.
    pub async fn prepare_controlled(
        config: ValidatedConfig,
    ) -> Result<(Self, RecordingController), RuntimeError> {
        let recorder = RecordingController {
            state: Arc::new(Mutex::new(RecordingState { active: None })),
        };
        if config.inputs.is_empty() {
            return Err(RuntimeError::NoInputs);
        }

        let destinations_by_input = compile_destinations(&config).await?;

        let mut inputs = Vec::with_capacity(config.inputs.len());
        for (input, destinations) in config.inputs.iter().zip(destinations_by_input) {
            let socket =
                UdpSocket::bind(input.bind)
                    .await
                    .map_err(|source| RuntimeError::BindInput {
                        input: input.id.to_string(),
                        address: input.bind,
                        source,
                    })?;
            inputs.push(PreparedInput {
                id: input.id.to_string(),
                socket,
                destinations: destinations.into(),
                recorder: recorder.clone(),
            });
        }

        Ok((
            Self {
                inputs,
                recorder: recorder.clone(),
            },
            recorder,
        ))
    }

    /// Runs until shutdown is requested or an input task fails.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if receive or send operations fail or if a
    /// forwarding task panics.
    pub async fn run_until<F>(self, shutdown: F) -> Result<(), RuntimeError>
    where
        F: Future<Output = io::Result<()>> + Send,
    {
        let recorder = self.recorder;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut tasks = JoinSet::new();

        for input in self.inputs {
            let mut shutdown_rx = shutdown_rx.clone();
            tasks.spawn(async move {
                let result = run_input(&input, &mut shutdown_rx).await;
                (input.id, result)
            });
        }
        drop(shutdown_rx);

        tokio::pin!(shutdown);
        let result = tokio::select! {
            shutdown_result = &mut shutdown => shutdown_result.map_err(RuntimeError::Shutdown),
            joined = tasks.join_next() => match joined {
                Some(Ok((input, Ok(())))) => Err(RuntimeError::InputTask {
                    input,
                    source: io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "forwarding task stopped before shutdown",
                    ),
                }),
                Some(Ok((input, Err(source)))) => Err(RuntimeError::InputTask { input, source }),
                Some(Err(error)) => Err(RuntimeError::TaskJoin(error)),
                None => Ok(()),
            }
        };

        let _ = shutdown_tx.send(true);
        while let Some(joined) = tasks.join_next().await {
            if result.is_ok() {
                match joined {
                    Ok((_, Ok(()))) => {}
                    Ok((input, Err(source))) => {
                        return Err(RuntimeError::InputTask { input, source });
                    }
                    Err(error) => return Err(RuntimeError::TaskJoin(error)),
                }
            }
        }
        if result.is_ok() && matches!(recorder.status()?, RecordingStatus::Recording(_)) {
            recorder.stop()?;
        }
        result
    }
}

async fn run_input(input: &PreparedInput, shutdown: &mut watch::Receiver<bool>) -> io::Result<()> {
    let mut buffer = vec![0_u8; MAX_DATAGRAM_SIZE];
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            received = input.socket.recv_from(&mut buffer) => {
                let (length, _) = received?;
                let payload = &buffer[..length];
                {
                    let mut state = input.recorder.state.lock()
                        .map_err(|_| io::Error::other("recording writer lock was poisoned"))?;
                    if let Some(active) = &mut state.active {
                        active.writer.write_datagram(&input.id, payload).map_err(io::Error::other)?;
                    }
                }
                for destination in input.destinations.iter() {
                    destination.socket.send_to(payload, destination.target).await?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use osc_config::parse_and_validate;
    use osc_recording::{Record, read_all};
    use tokio::{net::UdpSocket, sync::oneshot, time::timeout};

    use super::{ReplayTopology, Runtime, RuntimeError, replay};

    async fn free_address() -> std::net::SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("temporary socket should bind");
        socket
            .local_addr()
            .expect("temporary socket should have an address")
    }

    async fn receive(socket: &UdpSocket) -> Vec<u8> {
        let mut buffer = vec![0_u8; 65_507];
        let (length, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
            .await
            .expect("datagram should arrive before timeout")
            .expect("receiver should remain available");
        buffer.truncate(length);
        buffer
    }

    #[tokio::test]
    async fn forwards_payload_byte_identically() {
        let input = free_address().await;
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let output = receiver.local_addr().expect("receiver should have address");
        let config = parse_and_validate(&format!(
            "version = 1\n[[inputs]]\nid = \"source\"\nbind = \"{input}\"\n\
             [[outputs]]\nid = \"target\"\ntarget = \"{output}\"\n\
             [[mappings]]\nid = \"edge\"\nfrom = \"source\"\nto = \"target\"\n"
        ))
        .expect("configuration should be valid");
        let runtime = Runtime::prepare(config)
            .await
            .expect("runtime should prepare");
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(runtime.run_until(async {
            let _ = stop_rx.await;
            Ok(())
        }));

        let sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        let payload = b"#bundle\0raw-vmc-compatible-payload\0";
        sender
            .send_to(payload, input)
            .await
            .expect("sender should send");
        assert_eq!(receive(&receiver).await, payload);

        let _ = stop_tx.send(());
        task.await
            .expect("runtime task should not panic")
            .expect("runtime should stop cleanly");
    }

    #[tokio::test]
    async fn supports_fan_out() {
        let input = free_address().await;
        let first = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("first receiver should bind");
        let second = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("second receiver should bind");
        let config = parse_and_validate(&format!(
            "version = 1\n[[inputs]]\nid = \"source\"\nbind = \"{input}\"\n\
             [[outputs]]\nid = \"a\"\ntarget = \"{}\"\n\
             [[outputs]]\nid = \"b\"\ntarget = \"{}\"\n\
             [[mapping_groups]]\nid = \"fanout\"\nfrom = [\"source\"]\nto = [\"a\", \"b\"]\nmode = \"all-to-all\"\n",
            first.local_addr().expect("first receiver should have address"),
            second.local_addr().expect("second receiver should have address"),
        ))
        .expect("configuration should be valid");
        let runtime = Runtime::prepare(config)
            .await
            .expect("runtime should prepare");
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(runtime.run_until(async {
            let _ = stop_rx.await;
            Ok(())
        }));
        let sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        sender
            .send_to(b"fanout", input)
            .await
            .expect("sender should send");

        assert_eq!(receive(&first).await, b"fanout");
        assert_eq!(receive(&second).await, b"fanout");
        let _ = stop_tx.send(());
        task.await
            .expect("runtime task should not panic")
            .expect("runtime should stop cleanly");
    }

    #[tokio::test]
    async fn supports_multiple_inputs_converging_on_one_output() {
        let first_input = free_address().await;
        let second_input = free_address().await;
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let config = parse_and_validate(&format!(
            "version = 1\n\
             [[inputs]]\nid = \"a\"\nbind = \"{first_input}\"\n\
             [[inputs]]\nid = \"b\"\nbind = \"{second_input}\"\n\
             [[outputs]]\nid = \"target\"\ntarget = \"{}\"\n\
             [[mapping_groups]]\nid = \"converge\"\nfrom = [\"a\", \"b\"]\nto = [\"target\"]\nmode = \"all-to-all\"\n",
            receiver.local_addr().expect("receiver should have address"),
        ))
        .expect("configuration should be valid");
        let runtime = Runtime::prepare(config)
            .await
            .expect("runtime should prepare");
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(runtime.run_until(async {
            let _ = stop_rx.await;
            Ok(())
        }));
        let sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        sender
            .send_to(b"first", first_input)
            .await
            .expect("first datagram should send");
        assert_eq!(receive(&receiver).await, b"first");
        sender
            .send_to(b"second", second_input)
            .await
            .expect("second datagram should send");
        assert_eq!(receive(&receiver).await, b"second");

        let _ = stop_tx.send(());
        task.await
            .expect("runtime task should not panic")
            .expect("runtime should stop cleanly");
    }

    fn record(offset_ns: u64, stream: &str, payload: &[u8]) -> Record {
        Record {
            offset_ns,
            stream_id: stream.to_owned(),
            payload: payload.to_vec(),
        }
    }

    async fn receive_all(socket: &UdpSocket) -> Vec<Vec<u8>> {
        let mut got = Vec::new();
        let mut buffer = [0_u8; 64];
        while let Ok(Ok(length)) =
            tokio::time::timeout(Duration::from_millis(200), socket.recv(&mut buffer)).await
        {
            got.push(buffer[..length].to_vec());
        }
        got
    }

    #[tokio::test]
    async fn replay_restores_each_streams_mapping() {
        let alice_out = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let bob_out = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let both_out = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let config = parse_and_validate(&format!(
            "version = 1\n\
             [[inputs]]\nid = \"alice\"\nbind = \"127.0.0.1:0\"\n\
             [[inputs]]\nid = \"bob\"\nbind = \"127.0.0.1:0\"\n\
             [[outputs]]\nid = \"a\"\ntarget = \"{}\"\n\
             [[outputs]]\nid = \"b\"\ntarget = \"{}\"\n\
             [[outputs]]\nid = \"monitor\"\ntarget = \"{}\"\n\
             [[mappings]]\nid = \"alice-a\"\nfrom = \"alice\"\nto = \"a\"\n\
             [[mappings]]\nid = \"bob-b\"\nfrom = \"bob\"\nto = \"b\"\n\
             [[mappings]]\nid = \"bob-monitor\"\nfrom = \"bob\"\nto = \"monitor\"\n",
            alice_out.local_addr().expect("addr"),
            bob_out.local_addr().expect("addr"),
            both_out.local_addr().expect("addr"),
        ))
        .expect("config should validate");
        let topology = ReplayTopology::from_config(&config)
            .await
            .expect("topology should prepare");
        let records = vec![
            record(0, "alice", b"a1"),
            record(1_000, "bob", b"b1"),
            record(2_000, "stranger", b"s1"),
            record(3_000, "alice", b"a2"),
        ];
        assert!(topology.targets("stranger").is_empty());

        let sent = replay(&records, None, &topology, false)
            .await
            .expect("replay should run");
        assert_eq!(sent, 4); // a1, b1 x2 (b + monitor), a2; stranger skipped
        assert_eq!(
            receive_all(&alice_out).await,
            vec![b"a1".to_vec(), b"a2".to_vec()]
        );
        assert_eq!(receive_all(&bob_out).await, vec![b"b1".to_vec()]);
        assert_eq!(receive_all(&both_out).await, vec![b"b1".to_vec()]);

        let only_alice = ["alice".to_owned()];
        replay(&records, Some(&only_alice), &topology, false)
            .await
            .expect("replay should run");
        assert_eq!(receive_all(&alice_out).await.len(), 2);
        assert!(receive_all(&bob_out).await.is_empty());
    }

    #[tokio::test]
    async fn fan_out_sends_every_stream_to_every_target() {
        let first = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let second = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let topology = ReplayTopology::fan_out(&[
            first.local_addr().expect("addr"),
            second.local_addr().expect("addr"),
        ])
        .await
        .expect("topology should prepare");
        let records = vec![record(0, "x", b"x1"), record(10, "y", b"y1")];
        assert_eq!(
            replay(&records, None, &topology, false)
                .await
                .expect("replay should run"),
            4
        );
        for socket in [&first, &second] {
            assert_eq!(
                receive_all(socket).await,
                vec![b"x1".to_vec(), b"y1".to_vec()]
            );
        }
    }

    #[tokio::test]
    async fn records_input_datagrams_with_stream_identity() {
        let input = free_address().await;
        let path =
            std::env::temp_dir().join(format!("osc-runtime-recording-{}.oscr", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let config = parse_and_validate(&format!(
            "version = 1\n[[inputs]]\nid = \"vmc-face\"\nbind = \"{input}\"\n"
        ))
        .expect("configuration should be valid");
        let (runtime, recorder) = Runtime::prepare_controlled(config)
            .await
            .expect("runtime should prepare");
        recorder.start(&path).expect("recording should start");
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(runtime.run_until(async {
            let _ = stop_rx.await;
            Ok(())
        }));
        let sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        sender
            .send_to(b"recorded-vmc", input)
            .await
            .expect("datagram should send");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = stop_tx.send(());
        task.await
            .expect("runtime task should not panic")
            .expect("runtime should stop cleanly");

        let (_, records) = read_all(&path).expect("recording should be readable");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].stream_id, "vmc-face");
        assert_eq!(records[0].payload, b"recorded-vmc");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn prepare_fails_transactionally_when_an_input_is_busy() {
        let occupied = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("occupied socket should bind");
        let address = occupied
            .local_addr()
            .expect("occupied socket should have address");
        let config = parse_and_validate(&format!(
            "version = 1\n[[inputs]]\nid = \"busy\"\nbind = \"{address}\"\n"
        ))
        .expect("configuration should be structurally valid");

        assert!(matches!(
            Runtime::prepare(config).await,
            Err(RuntimeError::BindInput { .. })
        ));
    }
}
