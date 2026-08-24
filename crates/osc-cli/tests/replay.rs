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
