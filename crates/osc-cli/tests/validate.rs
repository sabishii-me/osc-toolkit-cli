use std::{fs, process::Command};

fn write_config(contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "osc-toolkit-cli-test-{}-{}.toml",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    fs::write(&path, contents).expect("test configuration should be writable");
    path
}

#[test]
fn validate_command_accepts_valid_configuration() {
    let path = write_config(
        r#"
version = 1

[[inputs]]
id = "source"
bind = "127.0.0.1:9000"

[[outputs]]
id = "target"
target = "127.0.0.1:10000"

[[mappings]]
id = "edge"
from = "source"
to = "target"
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["validate", "--config"])
        .arg(&path)
        .output()
        .expect("CLI should run");
    let _ = fs::remove_file(path);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "Configuration is valid: 1 inputs, 1 outputs, 1 mappings\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn validate_command_rejects_invalid_configuration() {
    let path = write_config("version = 999\n");

    let output = Command::new(env!("CARGO_BIN_EXE_osc-toolkit"))
        .args(["validate", "--config"])
        .arg(&path)
        .output()
        .expect("CLI should run");
    let _ = fs::remove_file(path);

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("unsupported configuration version 999")
    );
}
