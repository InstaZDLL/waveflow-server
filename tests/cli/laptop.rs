//! Laptop mode as a double-click starts it: the binary in a folder of its
//! own, beside the `laptop.env` the standalone archive ships
//! (`docs/plans/standalone-download-plan.md`, §5).
//!
//! These run the real executable, because what they hold still happens before
//! there is a router to call: which file is read, where the data goes, and
//! whether a second copy gets as far as the database.

use super::*;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long a debug build on a busy runner may take to boot.
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);

/// The server binary, linked into a folder of its own as an archive would
/// extract it.
///
/// Under `CARGO_TARGET_TMPDIR`, which is on the same volume as the binary, so
/// the link is a hard link and costs nothing; a copy only where that fails.
fn standalone_copy(root: &Path) -> PathBuf {
    let directory = root.join("waveflow-server");
    std::fs::create_dir_all(&directory).unwrap();
    let source = Path::new(env!("CARGO_BIN_EXE_waveflow-server"));
    let target = directory.join(source.file_name().unwrap());
    if std::fs::hard_link(source, &target).is_err() {
        std::fs::copy(source, &target).unwrap();
    }
    target
}

/// The `laptop.env` the archive ships, from the repository.
fn shipped_laptop_env() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packaging")
            .join("standalone")
            .join("laptop.env"),
    )
    .unwrap()
}

/// `program` with every place a system keeps application data pointed at
/// `home`, started in `working`.
fn laptop_command(program: &Path, home: &Path, working: &Path) -> Command {
    let mut command = command_for(program);
    command
        .env("LOCALAPPDATA", home)
        .env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env("RUST_LOG", "info")
        .current_dir(working)
        .stdin(Stdio::null());
    command
}

/// Where laptop mode keeps its data on this platform, under `home`.
fn os_data_dir(home: &Path) -> PathBuf {
    if cfg!(windows) {
        home.join("WaveFlow Server")
    } else if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("WaveFlow Server")
    } else {
        home.join(".local").join("share").join("waveflow-server")
    }
}

/// A server left running, read line by line, and killed — by its own handle,
/// never by name — when the test is done with it.
struct Running {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Running {
    fn spawn(mut command: Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// The first line containing `needle`, or a failure naming what came
    /// instead.
    fn line_containing(&self, needle: &str) -> String {
        let deadline = std::time::Instant::now() + BOOT_TIMEOUT;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line.contains(needle) => return line,
                Ok(line) => seen.push(line),
                Err(_) => panic!("no line containing {needle:?}; the server wrote {seen:#?}"),
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The port of `http://127.0.0.1:<port>/` in a banner.
fn port_in(banner: &str) -> u16 {
    let rest = banner
        .split("http://127.0.0.1:")
        .nth(1)
        .unwrap_or_else(|| panic!("no address in {banner:?}"));
    rest[..rest.find('/').unwrap()].parse().unwrap()
}

/// The whole journey of a download: `laptop.env` beside the binary turns
/// laptop mode on, the environment still wins over it, the data lands in the
/// system's folder and not beside the binary or in the working directory, the
/// banner says so — and a second double-click finds the first copy and stops
/// before it touches the database.
#[test]
fn a_double_clicked_server_keeps_its_data_in_the_os_folder_and_a_second_copy_stops() {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let program = standalone_copy(root.path());
    // The shipped file, plus a line the environment must override: were the
    // file to win, a bind that is not an address would refuse the boot.
    std::fs::write(
        program.with_file_name("laptop.env"),
        format!("{}\nWAVEFLOW_BIND=not-an-address\n", shipped_laptop_env()),
    )
    .unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    // A command line run from the same folder still owns its standard
    // output: `token create` promises its secret is alone there, and a line
    // about reading `laptop.env` ahead of it would break that for every
    // standalone install.
    let version = laptop_command(&program, &home, root.path())
        .arg("--version")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&version.stdout);
    assert!(version.status.success());
    assert_eq!(stdout.lines().count(), 1, "{stdout}");

    let mut command = laptop_command(&program, &home, root.path());
    command.env("WAVEFLOW_BIND", "127.0.0.1:0");
    let first = Running::spawn(command);
    let banner = first.line_containing("Close this window to stop the server");

    let data = os_data_dir(&home);
    assert!(
        banner.contains(&data.display().to_string()),
        "the banner names {} in {banner:?}",
        data.display()
    );
    assert!(data.join("waveflow.db").is_file());
    assert!(data.join("setup-token").is_file());
    assert!(!root.path().join("data").exists());
    assert!(!program.with_file_name("data").exists());
    let port = port_in(&banner);

    // An operator token makes the boot delete the drawn token file, so the
    // file still being there afterwards is the proof that the second copy
    // stopped before `initialize`.
    let second = laptop_command(&program, &home, root.path())
        .env("WAVEFLOW_BIND", format!("127.0.0.1:{port}"))
        .env(
            "WAVEFLOW_SETUP_TOKEN",
            "an operator token long enough to be accepted",
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        second.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        stdout.contains(&format!(
            "WaveFlow Server is already running at http://127.0.0.1:{port}/"
        )),
        "{stdout}"
    );
    assert!(data.join("setup-token").is_file());
}

/// A port another program holds is a refusal that names the port and the line
/// to change — said before the data folder is even created.
#[test]
fn a_port_held_by_another_program_is_refused_by_name() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    // Accepts and hangs up, as a program that does not speak HTTP would.
    std::thread::spawn(move || {
        for stream in holder.incoming() {
            drop(stream);
        }
    });

    let program = Path::new(env!("CARGO_BIN_EXE_waveflow-server"));
    let output = laptop_command(program, &home, root.path())
        .env("WAVEFLOW_LAPTOP", "true")
        .env("WAVEFLOW_BIND", format!("127.0.0.1:{port}"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains(&format!("cannot use port {port}")),
        "{stderr}"
    );
    assert!(
        stderr.contains("WAVEFLOW_BIND line of laptop.env"),
        "{stderr}"
    );
    assert!(!os_data_dir(&home).exists());
}

/// Without laptop mode, the same situation is the boot it always was: the
/// data in `./data` of the working directory, the database opened before the
/// port is tried, and no question asked of whoever holds it.
#[test]
fn an_installed_server_boots_as_it_always_did() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();

    let program = Path::new(env!("CARGO_BIN_EXE_waveflow-server"));
    let output = laptop_command(program, &home, root.path())
        .env("WAVEFLOW_BIND", format!("127.0.0.1:{port}"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains(&format!("bind failed on 127.0.0.1:{port}")),
        "{stderr}"
    );
    assert!(root.path().join("data").join("waveflow.db").is_file());
    assert!(!os_data_dir(&home).exists());
    drop(holder);
}
