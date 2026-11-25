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

/// A recording of three streams (`alice`, `bob`, `other`), as `serve` would write from
/// three inputs.
fn three_stream_recording(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "osc-toolkit-replay-{name}-{}.oscr",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut writer = Writer::create(&path).expect("recording should be created");
    for (stream, payload) in [
        ("alice", &b"a1"[..]),
        ("bob", b"b1"),
        ("other", b"o1"),
        ("alice", b"a2"),
    ] {
        writer
            .write_datagram(stream, payload)
            .expect("datagram should write");
    }
    writer.finish().expect("recording should finish");
    path
}

fn receiver() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("receiver should bind");
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout should be set");
    socket
}

fn address(socket: &UdpSocket) -> String {
    socket.local_addr().expect("address").to_string()
}

fn receive_all(socket: &UdpSocket) -> Vec<Vec<u8>> {
    let mut got = Vec::new();
    let mut buffer = [0_u8; 32];
    while let Ok(length) = socket.recv(&mut buffer) {
        got.push(buffer[..length].to_vec());
    }
    got
}

fn replay(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .arg("replay")
        .args(args)
        .output()
        .expect("replay should run")
}

#[test]
fn replay_config_restores_each_streams_mapping() {
    let path = three_stream_recording("config");
    let (alice, bob) = (receiver(), receiver());
    // The same kind of configuration `serve` records with: the recorded stream IDs are its
    // input IDs. `other` has no mapping and is skipped.
    let config = path.with_extension("toml");
    fs::write(
        &config,
        format!(
            "version = 1\n\
             [[inputs]]\nid = \"alice\"\nbind = \"127.0.0.1:0\"\n\
             [[inputs]]\nid = \"bob\"\nbind = \"127.0.0.1:0\"\n\
             [[inputs]]\nid = \"other\"\nbind = \"127.0.0.1:0\"\n\
             [[outputs]]\nid = \"alice-out\"\ntarget = \"{}\"\n\
             [[outputs]]\nid = \"bob-out\"\ntarget = \"{}\"\n\
             [[mappings]]\nid = \"a\"\nfrom = \"alice\"\nto = \"alice-out\"\n\
             [[mappings]]\nid = \"b\"\nfrom = \"bob\"\nto = \"bob-out\"\n",
            address(&alice),
            address(&bob)
        ),
    )
    .expect("config should write");
    let path_text = path.display().to_string();
    let config_text = config.display().to_string();
    let output = replay(&["--file", &path_text, "--config", &config_text]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("other: skipped (no mapping)"));
    assert_eq!(receive_all(&alice), vec![b"a1".to_vec(), b"a2".to_vec()]);
    assert_eq!(receive_all(&bob), vec![b"b1".to_vec()]);
    let _ = fs::remove_file(config);
    let _ = fs::remove_file(path);
}

#[test]
fn replay_refuses_to_merge_streams_into_a_target() {
    let path = three_stream_recording("merge-guard");
    let target = receiver();
    let path_text = path.display().to_string();
    let output = replay(&["--file", &path_text, "--target", &address(&target)]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("3 streams (alice, bob, other)"), "{error}");
    assert_eq!(
        receive_all(&target),
        Vec::<Vec<u8>>::new(),
        "nothing may be sent"
    );

    let merged = replay(&[
        "--file",
        &path_text,
        "--target",
        &address(&target),
        "--merge-streams",
    ]);
    assert!(merged.status.success(), "{merged:?}");
    assert_eq!(receive_all(&target).len(), 4);
    let _ = fs::remove_file(path);
}

#[test]
fn replay_stream_selects_one_stream_for_a_target() {
    let path = three_stream_recording("select");
    let target = receiver();
    let path_text = path.display().to_string();
    let output = replay(&[
        "--file",
        &path_text,
        "--stream",
        "alice",
        "--target",
        &address(&target),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(receive_all(&target), vec![b"a1".to_vec(), b"a2".to_vec()]);

    let unknown = replay(&[
        "--file",
        &path_text,
        "--stream",
        "nobody",
        "--target",
        &address(&target),
    ]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("`nobody` is not in the recording"));
    let _ = fs::remove_file(path);
}

#[test]
fn replay_requires_a_config_or_a_target() {
    let output = replay(&["--file", "missing.oscr"]);
    assert!(!output.status.success());
}
