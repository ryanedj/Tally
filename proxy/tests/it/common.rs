use proxy::{LogFile, LogWriter, log_queue, run_writer};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use tally_core::{Entry, LogLine, parse_log_line};
use tokio_util::sync::CancellationToken;

pub const RUN: &str = "8c1f0a3e9b7d4c62a5e01f9d3b7c4e18";
pub const CLIENT: &str = "weather-app";

pub fn start() -> Entry {
    Entry::Start {
        run: RUN.to_string(),
        settings_sha256: "9f2c".to_string(),
    }
}

pub fn pass() -> Entry {
    Entry::Pass {
        host: "api.weather.local:80".to_string(),
    }
}

/// The run's log file in "folder".
pub fn log_path(folder: &Path) -> PathBuf {
    folder.join(format!("{RUN}.jsonl"))
}

/// How a "FailingFile" fails once its countdown runs out.
#[derive(Debug, Clone, Copy)]
pub enum Failure {
    Append,
    ForceToDisk,
    Panic,
}

/// A log file that writes "lines_before" lines, then fails on the next one as "failure" says.
pub struct FailingFile {
    pub file: File,
    pub lines_before: usize,
    pub failure: Failure,
}

impl LogFile for FailingFile {
    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if self.lines_before == 0 {
            match self.failure {
                Failure::Append => return Err(io::Error::other("append fails on purpose")),
                Failure::Panic => panic!("append panics on purpose"),
                Failure::ForceToDisk => {}
            }
        }
        self.file.append(line)
    }

    fn force_to_disk(&mut self) -> io::Result<()> {
        if self.lines_before == 0 {
            return Err(io::Error::other("force_to_disk fails on purpose"));
        }
        self.lines_before -= 1;
        self.file.force_to_disk()
    }
}

/// A Writer for "CLIENT" on its own thread, writing to "file", with its "log_failed".
pub fn start_writer(
    file: impl LogFile + Send + 'static,
    capacity: usize,
) -> (LogWriter, CancellationToken, JoinHandle<()>) {
    let (writer, queue) = log_queue(capacity);
    let log_failed = CancellationToken::new();
    let token = log_failed.clone();
    let thread = std::thread::spawn(move || run_writer(file, queue, CLIENT.to_string(), token));
    (writer, log_failed, thread)
}

/// The lines of the log at "path", each ending in "\n" and read with "parse_log_line".
pub fn log_lines(path: &Path) -> Vec<LogLine> {
    let text = std::fs::read_to_string(path).expect("the log can be read");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "the last line ends in \\n"
    );
    text.lines()
        .map(|line| parse_log_line(line).expect("each line parses"))
        .collect()
}

/// A line's "seq", "client" and "time", the fields the Writer fills in.
pub fn filled_in(line: &LogLine) -> (u64, &str, &str) {
    match line {
        LogLine::Start {
            seq, client, time, ..
        }
        | LogLine::Release {
            seq, client, time, ..
        }
        | LogLine::Pass {
            seq, client, time, ..
        }
        | LogLine::Refuse {
            seq, client, time, ..
        }
        | LogLine::UpstreamError {
            seq, client, time, ..
        }
        | LogLine::Stop { seq, client, time } => (*seq, client, time),
    }
}
