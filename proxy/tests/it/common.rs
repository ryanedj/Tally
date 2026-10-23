use fake_api::{Arrival, Received};
use proxy::{
    Limits, LogFile, LogWriter, RunLog, Stopped, create_log_file, log_queue, run_writer, serve,
    start_log, stop_log, wait_for_writer,
};
use std::fs::File;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tally_core::{
    Entry, LogLine, Secrets, Settings, check_settings, parse_log_line, parse_secrets,
    parse_settings,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

pub const RUN: &str = "8c1f0a3e9b7d4c62a5e01f9d3b7c4e18";
pub const CLIENT: &str = "weather-app";

/// One host ("api.weather.local:80" at "127.0.0.1:9001"), one key ("weather_key", bound to
/// "Authorization").
pub const SETTINGS: &str = include_str!("../../../tally-core/tests/it/fixtures/one-key.toml");
pub const SECRETS: &str =
    include_str!("../../../tally-core/tests/it/fixtures/one-key-secrets.toml");
/// The header line carrying "weather_key"'s placeholder in its bound header.
pub const KEY: &str = "Authorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n";

/// Short limits for tests, each longer than the fake API's 500 ms pause in "/early-hints".
pub const LIMITS: Limits = Limits {
    idle: Duration::from_secs(2),
    head: Duration::from_secs(2),
    body: Duration::from_secs(2),
    connect: Duration::from_secs(2),
    write: Duration::from_secs(2),
    server_silence: Duration::from_secs(2),
    shutdown: Duration::from_secs(1),
    gentle_close: Duration::from_millis(300),
    end_of_run: Duration::from_secs(2),
};
/// For a test of one of the limits.
pub const SHORT: Duration = Duration::from_millis(300);

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

/// Each line without "seq", "client" and "time": "event [keys] host", then "about" and the
/// reason where the event has them, "null" for a missing host.
pub fn summary(lines: &[LogLine]) -> Vec<String> {
    lines
        .iter()
        .map(|line| match line {
            LogLine::Start { .. } => "start".to_string(),
            LogLine::Release { keys, host, .. } => {
                let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                format!("release [{}] {host}", keys.join(","))
            }
            LogLine::Pass { host, .. } => format!("pass {host}"),
            LogLine::Refuse {
                keys, host, reason, ..
            } => {
                let reason = serde_json::to_string(reason).expect("a reason serializes");
                let host = host.as_deref().unwrap_or("null");
                format!(
                    "refuse [{}] {host} {}",
                    keys.join(","),
                    reason.trim_matches('"')
                )
            }
            LogLine::UpstreamError {
                keys,
                host,
                about,
                reason,
                ..
            } => {
                let reason = serde_json::to_string(reason).expect("a reason serializes");
                let reason = reason.trim_matches('"');
                format!(
                    "upstream_error [{}] {host} about {about} {reason}",
                    keys.join(",")
                )
            }
            LogLine::Stop { .. } => "stop".to_string(),
        })
        .collect()
}

/// What the fake API has reported through its hook, in order, each with the number of the
/// connection it came on.
pub type Seen = Arc<Mutex<Vec<(u64, Arrival)>>>;

/// Starts a fake API on a free port, recording what reaches it.
pub async fn start_fake_api() -> (SocketAddr, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let address = listener.local_addr().expect("the listener has an address");
    let seen = Seen::default();
    let sink = Arc::clone(&seen);
    let hook = Arc::new(move |connection: u64, arrival: &Arrival| {
        sink.lock()
            .expect("the lock is not poisoned")
            .push((connection, arrival.clone()));
    });
    tokio::spawn(fake_api::serve(listener, hook));
    (address, seen)
}

/// Everything that has reached the fake API so far, each with its connection number.
pub fn arrivals(seen: &Seen) -> Vec<(u64, Arrival)> {
    seen.lock().expect("the lock is not poisoned").clone()
}

/// The complete requests that have reached the fake API, each with its connection number.
pub fn requests(seen: &Seen) -> Vec<(u64, Received)> {
    arrivals(seen)
        .into_iter()
        .filter_map(|(connection, arrival)| match arrival {
            Arrival::Request(received) => Some((connection, received)),
            Arrival::NotARequest(_) => None,
        })
        .collect()
}

/// A Server whose accept queue is full: Linux then drops new SYNs, so a connect hangs. Keep both
/// the listener and the queued connections alive.
#[cfg(target_os = "linux")]
pub async fn server_that_never_accepts() -> (TcpListener, Vec<TcpStream>) {
    let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
    socket
        .bind("127.0.0.1:0".parse().expect("an address"))
        .expect("bind a free port");
    let listener = socket.listen(0).expect("listen");
    let address = listener.local_addr().expect("the listener has an address");
    let mut queued = Vec::new();
    while let Ok(Ok(stream)) =
        timeout(Duration::from_millis(200), TcpStream::connect(address)).await
    {
        queued.push(stream);
    }
    (listener, queued)
}

/// The one-key settings, with the Server of "api.weather.local:80" at "server".
pub fn settings_for(server: SocketAddr) -> String {
    SETTINGS.replace("127.0.0.1:9001", &server.to_string())
}

/// A GET for "path" on "api.weather.local", with "headers" (each line ending in "\r\n") after
/// its "Host".
pub fn get(path: &str, headers: &str) -> Vec<u8> {
    format!(
        "GET http://api.weather.local{path} HTTP/1.1\r\nHost: api.weather.local\r\n{headers}\r\n"
    )
    .into_bytes()
}

/// Settings and secrets, from TOML text, past the startup rules.
pub fn checked(settings: &str, secrets: &str) -> (Settings, Secrets) {
    let secrets =
        parse_secrets(secrets.as_bytes(), Path::new("secrets.toml")).expect("the secrets parse");
    let raw = parse_settings(settings.as_bytes(), Path::new("settings.toml"))
        .expect("the settings parse");
    let settings = check_settings(raw, &secrets).expect("the settings pass the startup rules");
    (settings, secrets)
}

/// A proxy served in-process on a free port, logging to a temporary folder, as "main" runs it.
pub struct Proxy {
    pub address: SocketAddr,
    /// The run's log file.
    pub log: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    served: tokio::task::JoinHandle<(Result<(), Stopped>, tokio::time::Instant)>,
    run_log: RunLog,
    _folder: TempDir,
}

/// Starts a proxy with "settings" and "secrets" (TOML text) and "limits".
pub async fn start_proxy(settings: &str, secrets: &str, limits: Limits) -> Proxy {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    serve_in_process(settings, secrets, limits, file, folder).await
}

/// "start_proxy", with a log that fails on the line after the first "lines_before".
pub async fn start_failing_proxy(
    settings: &str,
    secrets: &str,
    limits: Limits,
    lines_before: usize,
) -> Proxy {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = FailingFile {
        file: create_log_file(folder.path(), RUN).expect("the log file is created"),
        lines_before,
        failure: Failure::Append,
    };
    serve_in_process(settings, secrets, limits, file, folder).await
}

async fn serve_in_process(
    settings: &str,
    secrets: &str,
    limits: Limits,
    file: impl LogFile + Send + 'static,
    folder: TempDir,
) -> Proxy {
    let (settings, secrets) = checked(settings, secrets);
    let run_log = start_log(
        file,
        CLIENT.to_string(),
        RUN.to_string(),
        "9f2c".to_string(),
    )
    .await
    .expect("the start line is written");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let address = listener.local_addr().expect("the listener has an address");
    let (shutdown, signal) = oneshot::channel::<()>();
    let served = tokio::spawn(serve(
        listener,
        settings,
        secrets,
        limits,
        run_log.writer.clone(),
        run_log.log_failed.clone(),
        async move {
            let _ = signal.await;
        },
    ));
    Proxy {
        address,
        log: log_path(folder.path()),
        shutdown: Some(shutdown),
        served,
        run_log,
        _folder: folder,
    }
}

impl Proxy {
    /// The log's lines, once it has at least "count". A connection's last line can land just
    /// after the program sees the connection close.
    pub async fn lines(&self, count: usize) -> Vec<LogLine> {
        for _ in 0..200 {
            let lines = log_lines(&self.log);
            if lines.len() >= count {
                return lines;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "still waiting for {count} lines after 2 s: {:?}",
            log_lines(&self.log)
        );
    }

    /// Sends the shutdown signal, then "ended".
    pub async fn stop(mut self) -> (Result<(), Stopped>, Vec<LogLine>) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.ended().await
    }

    /// Waits for "serve" to end, then ends the log as "main" does: what "serve" returned, and
    /// the log's lines.
    pub async fn ended(self) -> (Result<(), Stopped>, Vec<LogLine>) {
        let (served, deadline) = timeout(Duration::from_secs(10), self.served)
            .await
            .expect("serve ends within 10 s")
            .expect("serve doesn't panic");
        match served {
            Ok(()) => stop_log(self.run_log, deadline)
                .await
                .expect("the stop line is written"),
            Err(_) => wait_for_writer(self.run_log, deadline)
                .await
                .expect("the Writer ends in time"),
        }
        (served, log_lines(&self.log))
    }
}

/// The program's side of one connection to the proxy. Every read gives up after 5 s.
pub struct Program {
    stream: TcpStream,
    /// Bytes read but not taken yet.
    buffer: Vec<u8>,
}

impl Program {
    pub async fn connect(address: SocketAddr) -> Program {
        Program {
            stream: TcpStream::connect(address)
                .await
                .expect("connect to the proxy"),
            buffer: Vec::new(),
        }
    }

    pub async fn send(&mut self, bytes: &[u8]) {
        self.stream
            .write_all(bytes)
            .await
            .expect("send to the proxy");
    }

    /// Closes the program's side for writing; it can still read.
    pub async fn close_writing(&mut self) {
        self.stream
            .shutdown()
            .await
            .expect("close the writing side");
    }

    /// Reads more into "buffer"; false once the proxy has closed.
    async fn read_more(&mut self) -> bool {
        let read = timeout(
            Duration::from_secs(5),
            self.stream.read_buf(&mut self.buffer),
        )
        .await
        .expect("the proxy sends something within 5 s");
        matches!(read, Ok(read) if read > 0)
    }

    /// The next head the proxy sends, up to and including its empty line.
    pub async fn head(&mut self) -> String {
        loop {
            if let Some(end) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                let head: Vec<u8> = self.buffer.drain(..end + 4).collect();
                return String::from_utf8(head).expect("a head in text");
            }
            assert!(
                self.read_more().await,
                "the proxy closed before a whole head: {:?}",
                String::from_utf8_lossy(&self.buffer)
            );
        }
    }

    /// The next response: its head, and a body of its "content-length" (none if it has none).
    pub async fn response(&mut self) -> (String, Vec<u8>) {
        let head = self.head().await;
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .map_or(0, |length| length.parse().expect("a length"));
        while self.buffer.len() < length {
            assert!(self.read_more().await, "the proxy closed before the body");
        }
        let body = self.buffer.drain(..length).collect();
        (head, body)
    }

    /// Everything until the proxy closes the connection. A reset ends it like a close.
    pub async fn rest(&mut self) -> Vec<u8> {
        while self.read_more().await {}
        std::mem::take(&mut self.buffer)
    }

    /// Writes twice, 50 ms apart, then reads: true if both writes are taken and the read gives
    /// the end of the stream. A connection the proxy has fully closed answers the first write
    /// with a reset, so the second write or the read fails.
    pub async fn writes_are_still_taken(&mut self) -> bool {
        for _ in 0..2 {
            if self.stream.write_all(b"more").await.is_err() {
                return false;
            }
            sleep(Duration::from_millis(50)).await;
        }
        let mut byte = [0u8; 1];
        matches!(
            timeout(Duration::from_secs(1), self.stream.read(&mut byte)).await,
            Ok(Ok(0))
        )
    }
}

/// "request" gets a reply starting with "status", then a gentle close (DESIGN, Handling one
/// request, step 8): the end of the stream comes at once, well before the 2 s gentle-close limit,
/// while the proxy still takes what the program sends.
pub async fn assert_closes_gently(request: &[u8], status: &str) {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        gentle_close: Duration::from_secs(2),
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(request).await;
    let started = Instant::now();
    let reply = program.rest().await;
    assert!(
        String::from_utf8_lossy(&reply).starts_with(status),
        "{}",
        String::from_utf8_lossy(&reply)
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the proxy stops writing at once, not at the gentle-close limit"
    );
    assert!(
        program.writes_are_still_taken().await,
        "the proxy still takes what the program sends"
    );

    // The program's close ends the gentle close
    drop(program);
    assert_eq!(proxy.stop().await.0, Ok(()));
}
