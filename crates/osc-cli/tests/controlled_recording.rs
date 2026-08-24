use std::{
    fs,
    net::UdpSocket,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use osc_recording::read_all;

fn free_address() -> std::net::SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .expect("temporary socket should bind")
        .local_addr()
        .expect("temporary socket should have an address")
}

fn control(binary: &str, address: std::net::SocketAddr, args: &[&str]) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = Command::new(binary)
            .args(["record", "--control", &address.to_string()])
            .args(args)
            .output()
            .expect("control client should run");
        if output.status.success() {
            return String::from_utf8(output.stdout)
                .expect("response should be UTF-8")
                .trim()
                .to_owned();
        }
        assert!(Instant::now() < deadline, "control service did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn recording_start_and_stop_do_not_restart_listener() {
    let input = free_address();
    let control_address = free_address();
    let config = std::env::temp_dir().join(format!("osc-controlled-{}.toml", std::process::id()));
    let recording =
        std::env::temp_dir().join(format!("osc-controlled-{}.oscr", std::process::id()));
    let _ = fs::remove_file(&recording);
    fs::write(
        &config,
        format!("version = 1\n[[inputs]]\nid = \"port\"\nbind = \"{input}\"\n"),
    )
    .expect("configuration should write");
    let binary = env!("CARGO_BIN_EXE_osc-toolkit");
    let mut service = Command::new(binary)
        .args(["serve", "--config"])
        .arg(&config)
        .args(["--control", &control_address.to_string()])
        .stdout(Stdio::null())
        .spawn()
        .expect("service should start");

    assert_eq!(control(binary, control_address, &["status"]), "STATUS idle");
    let sender = UdpSocket::bind("127.0.0.1:0").expect("sender should bind");
    sender
        .send_to(b"before", input)
        .expect("before should send");
    let recording_text = recording.display().to_string();
    assert!(
        control(
            binary,
            control_address,
            &["start", "--file", &recording_text]
        )
        .starts_with("RECORDING")
    );
    sender
        .send_to(b"inside", input)
        .expect("inside should send");
    std::thread::sleep(Duration::from_millis(20));
    assert!(control(binary, control_address, &["stop"]).starts_with("FINALIZED"));
    sender.send_to(b"after", input).expect("after should send");
    assert_eq!(control(binary, control_address, &["status"]), "STATUS idle");
    assert_eq!(
        control(binary, control_address, &["shutdown"]),
        "SHUTDOWN accepted"
    );
    assert!(service.wait().expect("service should stop").success());

    let (_, records) = read_all(&recording).expect("recording should be valid");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].payload, b"inside");
    let _ = fs::remove_file(config);
    let _ = fs::remove_file(recording);
}
