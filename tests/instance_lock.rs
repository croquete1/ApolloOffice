use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn occupied_data_directory_rejects_server_before_database_open() {
    let fixture = Fixture(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "instance-lock-{}",
        hex::encode(rand::random::<[u8; 8]>())
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let lock_path = fixture.0.join("office.lock");
    let owner = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .unwrap();
    owner.try_lock().unwrap();

    let exe = env!("CARGO_BIN_EXE_apollo-office");
    let mut other = Command::new(exe)
        .arg("--data")
        .arg(&fixture.0)
        .args(["--port", "0", "--quiet"])
        .env_clear()
        .env("APOLLO_OFFICE_GH", fixture.0.join("missing-gh"))
        .env("APOLLO_OFFICE_CLAUDE", fixture.0.join("missing-claude"))
        .env("APOLLO_OFFICE_CODEX", fixture.0.join("missing-codex"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if other.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() >= Duration::from_secs(15) {
            let _ = other.kill();
            let _ = other.wait();
            panic!("second server did not reject occupied data directory");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = other.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("data directory is already in use"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.0.join("office.db").exists());

    // Maintenance commands remain usable while the server holds its lock.
    let status = Command::new(exe)
        .arg("--data")
        .arg(&fixture.0)
        .arg("status")
        .output()
        .unwrap();
    assert!(status.status.success(), "{:?}", status.stderr);
    assert!(fixture.0.join("office.db").exists());
    let contender = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(matches!(
        contender.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(owner);
    contender.try_lock().unwrap();
}
