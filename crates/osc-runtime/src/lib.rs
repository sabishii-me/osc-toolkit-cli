//! Bounded-memory UDP routing runtime.

use std::{future::Future, io, net::SocketAddr, sync::Arc};

use osc_config::ValidatedConfig;
use thiserror::Error;
use tokio::{net::UdpSocket, sync::watch, task::JoinSet};

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
    /// A forwarding task terminated unexpectedly.
    #[error("input task terminated unexpectedly: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

/// A prepared runtime whose sockets have all been created successfully.
pub struct Runtime {
    inputs: Vec<PreparedInput>,
}

struct PreparedInput {
    id: String,
    socket: UdpSocket,
    destinations: Arc<[Destination]>,
}

struct Destination {
    socket: Arc<UdpSocket>,
    target: SocketAddr,
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
        if config.inputs.is_empty() {
            return Err(RuntimeError::NoInputs);
        }

        let mut output_sockets = Vec::with_capacity(config.outputs.len());
        for output in &config.outputs {
            let bind_address = if output.target.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            };
            let socket = UdpSocket::bind(bind_address).await.map_err(|source| {
                RuntimeError::CreateOutput {
                    output: output.id.to_string(),
                    target: output.target,
                    source,
                }
            })?;
            output_sockets.push(Arc::new(socket));
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
            });
        }

        Ok(Self { inputs })
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
    use tokio::{net::UdpSocket, sync::oneshot, time::timeout};

    use super::{Runtime, RuntimeError};

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
