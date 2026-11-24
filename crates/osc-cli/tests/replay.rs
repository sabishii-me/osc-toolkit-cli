use std::{fs, net::UdpSocket, process::Command, time::Duration};

use osc_recording::Writer;

#[test]
fn replay_command_sends_recorded_datagrams() {
    let path = std::env::temp_dir().join(format!(
        "osc-toolkit-replay-test-{}.oscr",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut writer = Writer::create(&path).expect("recording should be created");
    writer
        .write_datagram("vmc", b"first")
        .expect("first datagram should write");
    std::thread::sleep(Duration::from_millis(5));
    writer
        .write_datagram("vmc", b"second")
        .expect("second datagram should write");
    writer.finish().expect("recording should finish");

    let receiver = UdpSocket::bind("127.0.0.1:0").expect("receiver should bind");
    receiver
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout should be set");
    let target = receiver
        .local_addr()
        .expect("receiver should have an address");
    let child = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["replay", "--file"])
        .arg(&path)
        .args(["--target", &target.to_string()])
        .spawn()
        .expect("replay should start");

    let mut buffer = [0_u8; 32];
    let first = receiver
        .recv(&mut buffer)
        .expect("first datagram should arrive");
    assert_eq!(&buffer[..first], b"first");
    let second = receiver
        .recv(&mut buffer)
        .expect("second datagram should arrive");
    assert_eq!(&buffer[..second], b"second");
    assert!(
        child
            .wait_with_output()
            .expect("replay should finish")
            .status
            .success()
    );
    let _ = fs::remove_file(path);
}

#[test]
fn replay_routes_each_stream_to_its_own_receiver() {
    let path = std::env::temp_dir().join(format!(
        "osc-toolkit-replay-route-test-{}.oscr",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut writer = Writer::create(&path).expect("recording should be created");
    writer
        .write_datagram("alice", b"a1")
        .expect("alice should write");
    writer
        .write_datagram("bob=2", b"b1")
        .expect("bob should write");
    writer
        .write_datagram("other", b"o1")
        .expect("other should write");
    writer
        .write_datagram("alice", b"a2")
        .expect("alice should write");
    writer.finish().expect("recording should finish");

    let bind = || {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("receiver should bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(1500)))
            .expect("timeout should be set");
        socket
    };
    let (alice, bob, fallback) = (bind(), bind(), bind());
    let address = |socket: &UdpSocket| socket.local_addr().expect("address").to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["replay", "--file"])
        .arg(&path)
        .args(["--route", &format!("alice={}", address(&alice))])
        .args(["--route", &format!("bob=2={}", address(&bob))])
        .args(["--target", &address(&fallback)])
        .output()
        .expect("replay should run");
    assert!(output.status.success(), "{output:?}");

    let receive_all = |socket: &UdpSocket| {
        let mut got = Vec::new();
        let mut buffer = [0_u8; 32];
        while let Ok(length) = socket.recv(&mut buffer) {
            got.push(buffer[..length].to_vec());
        }
        got
    };
    assert_eq!(receive_all(&alice), vec![b"a1".to_vec(), b"a2".to_vec()]);
    assert_eq!(receive_all(&bob), vec![b"b1".to_vec()]);
    assert_eq!(receive_all(&fallback), vec![b"o1".to_vec()]);
    let _ = fs::remove_file(path);
}

#[test]
fn replay_requires_a_target_or_a_route() {
    let output = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["replay", "--file", "missing.oscr"])
        .output()
        .expect("replay should run");
    assert!(!output.status.success());
}
