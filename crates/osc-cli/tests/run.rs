use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn free_address() -> std::net::SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("temporary socket should bind");
    socket
        .local_addr()
        .expect("temporary socket should have an address")
}

fn write_config(contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "osc-toolkit-run-test-{}-{}.toml",
        std::process::id(),
        free_address().port()
    ));
    fs::write(&path, contents).expect("test configuration should be writable");
    path
}

fn wait_for_forwarding(sender: &UdpSocket, receiver: &UdpSocket, input: std::net::SocketAddr) {
    receiver
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout should be configurable");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buffer = [0_u8; 128];
    loop {
        sender
            .send_to(b"vmc-passthrough", input)
            .expect("test datagram should send");
        match receiver.recv_from(&mut buffer) {
            Ok((length, _)) => {
                assert_eq!(&buffer[..length], b"vmc-passthrough");
                return;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("receiver failed: {error}"),
        }
        assert!(Instant::now() < deadline, "CLI did not start forwarding");
    }
}

fn stop_child(mut child: Child) {
    child.kill().expect("test process should stop");
    let status = child.wait().expect("test process should be reaped");
    assert!(!status.success(), "forced test shutdown should be non-zero");
}

#[test]
fn run_command_forwards_udp_datagrams() {
    let input = free_address();
    let receiver = UdpSocket::bind("127.0.0.1:0").expect("receiver should bind");
    let output = receiver
        .local_addr()
        .expect("receiver should have an address");
    let config = write_config(&format!(
        "version = 1\n[[inputs]]\nid = \"source\"\nbind = \"{input}\"\n\
         [[outputs]]\nid = \"target\"\ntarget = \"{output}\"\n\
         [[mappings]]\nid = \"edge\"\nfrom = \"source\"\nto = \"target\"\n"
    ));

    let child = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["run", "--config"])
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("CLI should start");

    let sender = UdpSocket::bind("127.0.0.1:0").expect("sender should bind");
    wait_for_forwarding(&sender, &receiver, input);
    stop_child(child);
    let _ = fs::remove_file(config);
    thread::sleep(Duration::from_millis(20));
}
