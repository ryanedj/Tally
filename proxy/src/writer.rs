use jiff::Timestamp;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::thread::JoinHandle;
use std::time::SystemTime;
use tally_core::{Entry, LogLine};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Where the Writer puts its lines: the run's file, or a stand-in in tests.
pub trait LogFile {
    /// Appends "line", which ends in "\n".
    fn append(&mut self, line: &[u8]) -> io::Result<()>;
    /// Returns once everything appended so far is on disk.
    fn force_to_disk(&mut self) -> io::Result<()>;
}

impl LogFile for File {
    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        self.write_all(line)
    }

    fn force_to_disk(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

/// The Writer's side of the queue. Each line comes with where to send its "seq" once it's on
/// disk. Dropping that sender answers "Error".
pub type LogQueue = mpsc::Receiver<(Entry, oneshot::Sender<u64>)>;

/// The connections' side of the queue to the Writer. Clones share the same queue.
#[derive(Debug, Clone)]
pub struct LogWriter(mpsc::Sender<(Entry, oneshot::Sender<u64>)>);

/// The Writer answered "Error", or has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the log can't be written")]
pub struct LogFailed;

impl LogWriter {
    /// Hands "entry" to the Writer and waits until its line is on disk, then gives its "seq".
    /// When the queue is full, waits for room first: a line is never dropped.
    pub async fn submit(&self, entry: Entry) -> Result<u64, LogFailed> {
        let (answer_to, answer) = oneshot::channel();
        self.0
            .send((entry, answer_to))
            .await
            .map_err(|_| LogFailed)?;
        answer.await.map_err(|_| LogFailed)
    }
}

/// A queue of at most "capacity" lines, from the connections to the Writer. Panics if
/// "capacity" is 0 (tokio's "mpsc::channel").
pub fn log_queue(capacity: usize) -> (LogWriter, LogQueue) {
    let (sender, receiver) = mpsc::channel(capacity);
    (LogWriter(sender), receiver)
}

/// The Writer (2. Log writer). Writes each line of "queue" to "file" in arrival order, and
/// answers once it's on disk. Returns once the stop line is on disk, answering "Error" to any
/// line after it, or when every "LogWriter" is dropped and the queue is empty. On the first
/// error, answers "Error" to that line and every queued one, prints the error and sets
/// "log_failed"; a panic sets it too. It blocks, so it runs on its own thread.
pub fn run_writer<F: LogFile>(
    mut file: F,
    queue: LogQueue,
    client: String,
    log_failed: CancellationToken,
) {
    // Sets "log_failed" on every way out except the two normal ends, a panic included
    let guard = log_failed.drop_guard();
    // Declared after the guard so that on a panic it's dropped first: locals drop in reverse
    // order, so the queued lines get "Error" before "log_failed" is set, as in step 4
    let mut queue = queue;
    let mut next_seq = 0;

    // 1. Take the next line, in arrival order
    while let Some((entry, answer_to)) = queue.blocking_recv() {
        let is_stop = matches!(entry, Entry::Stop);

        // 2. Complete it
        let written = Timestamp::try_from(SystemTime::now())
            .map_err(io::Error::other)
            .and_then(|now| {
                let (seq, client, time) = (next_seq, client.clone(), format!("{now:.0}"));
                let line = match entry {
                    Entry::Start {
                        run,
                        settings_sha256,
                    } => LogLine::Start {
                        seq,
                        run,
                        client,
                        settings_sha256,
                        time,
                    },
                    Entry::Release { keys, host } => LogLine::Release {
                        seq,
                        client,
                        keys,
                        host,
                        time,
                    },
                    Entry::Pass { host } => LogLine::Pass {
                        seq,
                        client,
                        keys: Vec::new(),
                        host,
                        time,
                    },
                    Entry::Refuse { keys, host, reason } => LogLine::Refuse {
                        seq,
                        client,
                        keys,
                        host,
                        reason,
                        time,
                    },
                    Entry::UpstreamError {
                        keys,
                        host,
                        about,
                        reason,
                    } => LogLine::UpstreamError {
                        seq,
                        client,
                        keys,
                        host,
                        about,
                        reason,
                        time,
                    },
                    Entry::Stop => LogLine::Stop { seq, client, time },
                };

                // 3. Append it and force it to disk
                let mut bytes = serde_json::to_vec(&line)?;
                bytes.push(b'\n');
                file.append(&bytes)?;
                file.force_to_disk()
            });

        if let Err(error) = written {
            // 4. Print the error, stop the proxy. Dropping an answer's sender answers "Error".
            drop(answer_to);
            queue.close();
            while queue.blocking_recv().is_some() {}
            eprintln!("the log can't be written: {error}");
            return;
        }
        // Fails only if the connection stopped waiting; its line is on disk all the same
        let _ = answer_to.send(next_seq);
        next_seq += 1;

        if is_stop {
            // The stop line is the run's last: whatever is queued after it, or sent later, gets
            // "Error" and is never written
            queue.close();
            while queue.blocking_recv().is_some() {}
            break;
        }
    }
    guard.disarm();
}

/// Creates "<folder>/<run>.jsonl", never over an existing file, and "folder" first if it's
/// missing (one level). On Unix, the names are forced to disk too (fsync(2)); SQLite doesn't do
/// it on Windows either.
pub fn create_log_file(folder: &Path, run: &str) -> io::Result<File> {
    if let Err(error) = std::fs::create_dir(folder)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error);
    }
    #[cfg(unix)]
    let folder_handle = {
        let parent = folder
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
        File::open(folder)?
    };
    let file = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(folder.join(format!("{run}.jsonl")))?;
    // The file's name is on disk once its folder is synced. fsync covers the changes made after
    // the folder was opened.
    #[cfg(unix)]
    folder_handle.sync_all()?;
    Ok(file)
}

/// The run's log once its Writer runs: the queue's side the connections clone, the flag the
/// Writer sets when the log fails, and the Writer's thread.
#[derive(Debug)]
pub struct RunLog {
    pub writer: LogWriter,
    pub log_failed: CancellationToken,
    thread: JoinHandle<()>,
}

/// Why the run's log couldn't start (1. Proxy startup, exits).
#[derive(Debug, thiserror::Error)]
pub enum StartLogError {
    #[error("the Writer's thread can't be started: {0}")]
    Thread(io::Error),
    #[error("the log can't be written")]
    StartLine,
}

/// Step 4 of 1. Proxy startup, once the run's file exists: starts the Writer on its own thread
/// and logs the start line. If the line fails, waits for the Writer to end first, so its error is
/// printed before the proxy exits.
pub async fn start_log(
    file: impl LogFile + Send + 'static,
    client: String,
    run: String,
    settings_sha256: String,
) -> Result<RunLog, StartLogError> {
    let (writer, queue) = log_queue(1024);
    let log_failed = CancellationToken::new();
    let token = log_failed.clone();
    let thread = std::thread::Builder::new()
        .name("log-writer".to_string())
        .spawn(move || run_writer(file, queue, client, token))
        .map_err(StartLogError::Thread)?;
    if writer
        .submit(Entry::Start {
            run,
            settings_sha256,
        })
        .await
        .is_err()
    {
        drop(writer);
        let _ = thread.join();
        return Err(StartLogError::StartLine);
    }
    Ok(RunLog {
        writer,
        log_failed,
        thread,
    })
}

/// Step 6 of 1. Proxy startup: logs the stop line after every line already queued, then waits
/// for the Writer to end, whether the line was written or not. The Writer ends at the stop line
/// even if a clone of "log.writer" is still alive; a line that clone sends later gets "LogFailed".
pub async fn stop_log(log: RunLog) -> Result<(), LogFailed> {
    let stopped = log.writer.submit(Entry::Stop).await;
    drop(log.writer);
    let joined = log.thread.join();
    stopped?;
    joined.map_err(|_| LogFailed)
}
