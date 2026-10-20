// 1. Proxy startup, one test per row of its exits table. The rows the operator can cause run the
// real binary and check what it prints and its exit code. The rows a failing disk causes
// call "start_log" and "stop_log" with a "FailingFile".

use crate::common::{CLIENT, FailingFile, Failure, RUN, log_lines, log_path};
use proxy::{LogFailed, LogFile, StartLogError, create_log_file, start_log, stop_log};
use std::io;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tally_core::LogLine;
use tempfile::TempDir;

const SETTINGS: &str = include_str!("../../../tally-core/tests/it/fixtures/one-key.toml");
const SECRETS: &str = include_str!("../../../tally-core/tests/it/fixtures/one-key-secrets.toml");

/// A folder holding "settings.toml" and "secrets.toml", with these contents.
fn folder_with(settings: &str, secrets: &str) -> TempDir {
    let folder = tempfile::tempdir().expect("a temporary folder");
    std::fs::write(folder.path().join("settings.toml"), settings).expect("the settings file");
    std::fs::write(folder.path().join("secrets.toml"), secrets).expect("the secrets file");
    folder
}

/// Runs the proxy binary in "folder" with "args", as the operator would.
fn run_proxy(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_proxy"))
        .args(args)
        .current_dir(folder)
        .output()
        .expect("the proxy binary runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A "FailingFile" that sets "dropped" when the Writer drops it, 100 ms late. The Writer drops
/// its file only after printing its error and returning, so a set flag means the error was
/// printed. The delay makes a caller that doesn't wait for the Writer return before it's set.
struct WatchedFile {
    file: FailingFile,
    dropped: Arc<AtomicBool>,
}

impl LogFile for WatchedFile {
    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        self.file.append(line)
    }

    fn force_to_disk(&mut self) -> io::Result<()> {
        self.file.force_to_disk()
    }
}

impl Drop for WatchedFile {
    fn drop(&mut self) {
        std::thread::sleep(Duration::from_millis(100));
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn the_command_line_doesnt_name_two_files() {
    let folder = folder_with(SETTINGS, SECRETS);
    let output = run_proxy(folder.path(), &["settings.toml"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("usage: proxy <settings file> <secrets file>"),
        "{}",
        stderr(&output)
    );
    assert!(!folder.path().join("logs").exists(), "no log folder");
}

#[test]
fn a_file_that_cant_be_read() {
    let folder = folder_with(SETTINGS, SECRETS);
    let output = run_proxy(folder.path(), &["missing.toml", "secrets.toml"]);
    assert_eq!(output.status.code(), Some(1));
    let printed = stderr(&output);
    assert!(
        printed.contains("missing.toml") && printed.contains("can't be read"),
        "{printed}"
    );
    assert!(!folder.path().join("logs").exists(), "no log folder");
}

#[test]
fn a_file_that_isnt_valid_toml() {
    let folder = folder_with(SETTINGS, "weather_key = the-real-key\n");
    let output = run_proxy(folder.path(), &["settings.toml", "secrets.toml"]);
    assert_eq!(output.status.code(), Some(1));
    let printed = stderr(&output);
    assert!(
        printed.contains("not valid TOML for secrets, at line 1"),
        "{printed}"
    );
    assert!(
        !printed.contains("the-real-key"),
        "the secret isn't printed"
    );
    assert!(!folder.path().join("logs").exists(), "no log folder");
}

#[test]
fn every_broken_startup_rule_is_printed() {
    let settings = SETTINGS.replace("TALLY_0123456789abcdef0123456789abcdef", "TALLY_bad");
    let secrets = "weather_key = \"has a space\"\nother_key = \"x\"\n";
    let folder = folder_with(&settings, secrets);
    let output = run_proxy(folder.path(), &["settings.toml", "secrets.toml"]);
    assert_eq!(output.status.code(), Some(1));
    let printed = stderr(&output);
    for problem in [
        "key weather_key: malformed placeholder",
        "secret weather_key: invalid value",
        "secret other_key: no such key",
    ] {
        assert!(printed.contains(problem), "{problem} in: {printed}");
    }
    assert!(!printed.contains("has a space"), "the secret isn't printed");
    assert!(!folder.path().join("logs").exists(), "no log folder");
}

#[test]
fn the_log_file_cant_be_created() {
    let settings = SETTINGS.replace("log_folder = \"logs\"", "log_folder = \"missing/logs\"");
    let folder = folder_with(&settings, SECRETS);
    let output = run_proxy(folder.path(), &["settings.toml", "secrets.toml"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("this run's log file can't be created"),
        "{}",
        stderr(&output)
    );
}

#[tokio::test]
async fn the_start_line_cant_be_written() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let dropped = Arc::new(AtomicBool::new(false));
    let watched = WatchedFile {
        file: FailingFile {
            file,
            lines_before: 0,
            failure: Failure::Append,
        },
        dropped: Arc::clone(&dropped),
    };
    let started = start_log(
        watched,
        CLIENT.to_string(),
        RUN.to_string(),
        "9f2c".to_string(),
    )
    .await;
    match started {
        Err(error @ StartLogError::StartLine) => {
            assert_eq!(error.to_string(), "the log can't be written")
        }
        other => panic!("expected StartLine, got {other:?}"),
    }
    assert!(
        dropped.load(Ordering::SeqCst),
        "start_log returns only once the Writer has printed its error and ended"
    );
    assert!(log_lines(&log_path(folder.path())).is_empty());
}

#[tokio::test]
async fn the_start_line_can_stay_in_the_file_when_only_its_sync_fails() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 0,
        failure: Failure::ForceToDisk,
    };
    let started = start_log(
        failing,
        CLIENT.to_string(),
        RUN.to_string(),
        "9f2c".to_string(),
    )
    .await;
    assert!(
        matches!(started, Err(StartLogError::StartLine)),
        "{started:?}"
    );
    let lines = log_lines(&log_path(folder.path()));
    assert!(
        matches!(lines.as_slice(), [LogLine::Start { .. }]),
        "the written start line stays: {lines:?}"
    );
}

#[tokio::test]
async fn the_stop_line_cant_be_written() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let dropped = Arc::new(AtomicBool::new(false));
    let watched = WatchedFile {
        file: FailingFile {
            file,
            lines_before: 1,
            failure: Failure::Append,
        },
        dropped: Arc::clone(&dropped),
    };
    let log = start_log(
        watched,
        CLIENT.to_string(),
        RUN.to_string(),
        "9f2c".to_string(),
    )
    .await
    .expect("the start line is written");
    let log_failed = log.log_failed.clone();

    assert_eq!(stop_log(log).await, Err(LogFailed));
    assert!(
        dropped.load(Ordering::SeqCst),
        "stop_log returns only once the Writer has printed its error and ended"
    );
    assert!(log_failed.is_cancelled());
    let lines = log_lines(&log_path(folder.path()));
    assert!(
        matches!(lines.as_slice(), [LogLine::Start { .. }]),
        "no stop line: {lines:?}"
    );
}

#[tokio::test]
async fn the_stop_line_can_stay_in_the_file_when_only_its_sync_fails() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 1,
        failure: Failure::ForceToDisk,
    };
    let log = start_log(
        failing,
        CLIENT.to_string(),
        RUN.to_string(),
        "9f2c".to_string(),
    )
    .await
    .expect("the start line is written");

    assert_eq!(stop_log(log).await, Err(LogFailed));
    let lines = log_lines(&log_path(folder.path()));
    assert!(
        matches!(
            lines.as_slice(),
            [LogLine::Start { .. }, LogLine::Stop { .. }]
        ),
        "the written stop line stays, though the proxy exits with a failure: {lines:?}"
    );
}

// Until L5 adds the listen loop, a run logs its start line, then its stop line
#[test]
fn a_run_logs_its_start_line_then_its_stop_line() {
    let folder = folder_with(SETTINGS, SECRETS);
    let output = run_proxy(folder.path(), &["settings.toml", "secrets.toml"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));

    let logs: Vec<_> = std::fs::read_dir(folder.path().join("logs"))
        .expect("the log folder is created")
        .map(|entry| entry.expect("a log folder entry").path())
        .collect();
    let [path] = logs.as_slice() else {
        panic!("expected one log file, got {logs:?}");
    };
    let run = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".jsonl"))
        .expect("a file named <run>.jsonl");
    assert!(
        run.len() == 32 && run.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "32 lowercase hex characters: {run}"
    );
    match log_lines(path).as_slice() {
        [
            LogLine::Start {
                seq: 0,
                run: logged_run,
                client,
                settings_sha256,
                ..
            },
            LogLine::Stop { seq: 1, .. },
        ] => {
            assert_eq!(logged_run, run);
            assert_eq!(client, CLIENT);
            assert_eq!(
                settings_sha256,
                &tally_core::settings_sha256(SETTINGS.as_bytes())
            );
        }
        other => panic!("expected a start line then a stop line, got {other:?}"),
    }
}
