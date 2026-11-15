use std::process::{Command, Stdio};

#[test]
fn log_into_closed_pipe_exits_quietly() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let logs = if cfg!(target_os = "macos") {
        home.path().join("Library/Logs/shmc")
    } else {
        state.join("shmc")
    };
    std::fs::create_dir_all(&logs).unwrap();
    // Far more than a pipe buffer holds, so a write fails once the reader is gone.
    std::fs::write(logs.join("shmc.log"), "a log line\n".repeat(100_000)).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_shmc"))
        .args(["log", "100000"])
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", &state)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && stderr.is_empty(), "{:?}: {stderr}", out.status);
}

#[test]
fn runtime_error_has_the_usage_error_prefix() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config.toml");
    std::fs::write(&config, "[plex]\nserver = 3\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_shmc"))
        .arg("--config")
        .arg(&config)
        .args(["config", "--list"])
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.starts_with("shmc: "), "{stderr}");
}

#[test]
fn version_flags_print_the_version() {
    for flag in ["--version", "-V", "-v"] {
        let out = Command::new(env!("CARGO_BIN_EXE_shmc")).arg(flag).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{flag}: {:?}", out.status);
        assert!(
            stdout.starts_with(concat!("shmc ", env!("CARGO_PKG_VERSION"))),
            "{flag}: {stdout}"
        );
    }
}
