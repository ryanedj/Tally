use crate::connection::handle_connection;
use crate::writer::LogWriter;
use std::future::Future;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tally_core::{Secrets, Settings};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout_at};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// What a connection to the Server must do. Behind a trait object, so that a test can hand in a
/// connection that breaks or stalls where it chooses: over loopback, the system takes a whole
/// request before a real Server can do either.
pub(crate) trait ServerStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> ServerStream for T {}

/// Opens a connection to the Server at an address from the settings, "serve" uses
/// "TcpStream::connect", tests a stand-in.
pub(crate) type Connect = Box<
    dyn Fn(SocketAddr) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn ServerStream>>> + Send>>
        + Send
        + Sync,
>;

/// The time limits of DESIGN's Timeouts, of the shutdown and of the gentle close. "main" passes
/// DESIGN's values, tests pass shorter ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Silence before a request's first byte.
    pub idle: Duration,
    /// The whole head, from its first byte.
    pub head: Duration,
    /// The whole body.
    pub body: Duration,
    /// Connecting to the Server.
    pub connect: Duration,
    /// A write to the program that makes no progress.
    pub write: Duration,
    /// Nothing from the Server: a write to it that makes no progress, or no byte of its response.
    pub server_silence: Duration,
    /// From the shutdown signal to the cut-off.
    pub shutdown: Duration,
    /// Reading and discarding what the program still sends, before a close.
    pub gentle_close: Duration,
    /// From the cut-off to the end of the run: the connections still waiting for the Writer, the
    /// stop line and the Writer's end.
    pub end_of_run: Duration,
}

/// Why the run ends without its stop line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Stopped {
    /// The Writer failed or panicked, and printed why.
    #[error("the log can't be written")]
    LogFailed,
    /// A connection panicked, the run ends like a crash.
    #[error("a connection panicked, the run ends without its stop line")]
    ConnectionPanicked,
    /// The end of the run took longer than its limit: a stuck disk.
    #[error("the log didn't answer in time")]
    LogTimedOut,
}

/// What every connection is given (4. One program connection, inputs).
pub(crate) struct Shared {
    pub(crate) settings: Settings,
    pub(crate) secrets: Secrets,
    pub(crate) limits: Limits,
    pub(crate) writer: LogWriter,
    /// Set by the Writer when the log fails, or by the connection that learns it first: its
    /// answer comes before the Writer sets it.
    pub(crate) log_failed: CancellationToken,
    pub(crate) connect: Connect,
    /// Set by the shutdown signal: a connection finishes its current request, then closes.
    pub(crate) shutdown: CancellationToken,
    /// Set at the shutdown limit, or at once when the run ends early: a connection stops at its
    /// next wait.
    pub(crate) cut_off: CancellationToken,
}

/// Steps 6 and 7 of 1. Proxy startup, up to the stop line: serves each connection accepted on
/// "listener" in its own task (4. One program connection) until "shutdown_signal" completes, then
/// shuts down. "Ok" after a shutdown; an error if the log failed or stopped answering, or a
/// connection panicked, which ends the run without a stop line. With it, the end-of-run deadline:
/// the stop line and the Writer's end must be done by then too.
pub async fn serve(
    listener: TcpListener,
    settings: Settings,
    secrets: Secrets,
    limits: Limits,
    writer: LogWriter,
    log_failed: CancellationToken,
    shutdown_signal: impl Future<Output = ()>,
) -> (Result<(), Stopped>, Instant) {
    let shared = Arc::new(Shared {
        settings,
        secrets,
        limits,
        writer,
        log_failed,
        connect: Box::new(|address| {
            Box::pin(async move {
                let server: Box<dyn ServerStream> = Box::new(TcpStream::connect(address).await?);
                Ok(server)
            })
        }),
        shutdown: CancellationToken::new(),
        cut_off: CancellationToken::new(),
    });
    serve_with(
        listener,
        shared,
        shutdown_signal,
        |program, shared| async move { handle_connection(program, &shared).await },
    )
    .await
}

/// "serve", with what each connection runs passed in, so that a unit test can make one panic.
async fn serve_with<C, F>(
    listener: TcpListener,
    shared: Arc<Shared>,
    shutdown_signal: impl Future<Output = ()>,
    connection: C,
) -> (Result<(), Stopped>, Instant)
where
    C: Fn(TcpStream, Arc<Shared>) -> F,
    F: Future<Output = ()> + Send + 'static,
{
    let log_failed = &shared.log_failed;
    let tracker = TaskTracker::new();
    let panicked = CancellationToken::new();
    let mut shutdown_signal = std::pin::pin!(shutdown_signal);

    // 6. Serve. A panic ends the run gently, like the shutdown signal; a failed log at once.
    let gently = loop {
        tokio::select! {
            () = &mut shutdown_signal => break true,
            () = panicked.cancelled() => break true,
            () = log_failed.cancelled() => break false,
            program = async {
                // As axum does: an error about that one connection is retried at once, any other
                // after 1 s. The wait is inside this future, so the other three are still watched.
                loop {
                    match listener.accept().await {
                        Ok((program, _)) => break program,
                        Err(error) => {
                            eprintln!("accept error: {error}");
                            if !matches!(
                                error.kind(),
                                ErrorKind::ConnectionRefused
                                    | ErrorKind::ConnectionAborted
                                    | ErrorKind::ConnectionReset
                            ) {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            } => {
                // Set if the task ends any other way than by returning: a panic
                let guard = panicked.clone().drop_guard();
                let task = connection(program, Arc::clone(&shared));
                tracker.spawn(async move {
                    task.await;
                    guard.disarm();
                });
            }
        }
    };

    // 7. Shut down
    drop(listener);
    tracker.close();
    if gently {
        shared.shutdown.cancel();
        tokio::select! {
            () = tracker.wait() => {}
            () = tokio::time::sleep(shared.limits.shutdown) => {}
            () = log_failed.cancelled() => {}
        }
    }
    // The rest stop at their next wait (see 4). A wait for the Writer's answer doesn't, so from
    // here the end of the run has a limit: a stuck disk can't hold it forever
    shared.cut_off.cancel();
    let deadline = Instant::now() + shared.limits.end_of_run;
    let ended = if timeout_at(deadline, tracker.wait()).await.is_err() {
        Err(Stopped::LogTimedOut)
    } else if log_failed.is_cancelled() {
        Err(Stopped::LogFailed)
    } else if panicked.is_cancelled() {
        Err(Stopped::ConnectionPanicked)
    } else {
        Ok(())
    };
    (ended, deadline)
}

/// For unit tests: what a connection is given, with the one-key settings (the key
/// "weather_key", bound to "Authorization", toward "api.weather.local:80").
#[cfg(test)]
pub(crate) fn shared_for_tests(limits: Limits, writer: LogWriter, connect: Connect) -> Shared {
    use std::path::Path;
    use tally_core::{check_settings, parse_secrets, parse_settings};

    let secrets = parse_secrets(
        include_bytes!("../../tally-core/tests/it/fixtures/one-key-secrets.toml"),
        Path::new("secrets.toml"),
    )
    .expect("the secrets parse");
    let raw = parse_settings(
        include_bytes!("../../tally-core/tests/it/fixtures/one-key.toml"),
        Path::new("settings.toml"),
    )
    .expect("the settings parse");
    let settings = check_settings(raw, &secrets).expect("the settings pass the startup rules");
    Shared {
        settings,
        secrets,
        limits,
        writer,
        log_failed: CancellationToken::new(),
        connect,
        shutdown: CancellationToken::new(),
        cut_off: CancellationToken::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::log_queue;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // A panic needs a bug, so only this private seam can cause one (DESIGN, Tests)
    #[tokio::test]
    async fn a_panicking_connection_ends_the_run_once_the_others_are_done() {
        let (writer, _queue) = log_queue(16);
        let limit = Duration::from_millis(300);
        let limits = Limits {
            idle: limit,
            head: limit,
            body: limit,
            connect: limit,
            write: limit,
            server_silence: limit,
            shutdown: Duration::from_secs(5),
            gentle_close: limit,
            end_of_run: Duration::from_secs(5),
        };
        // No connection here reaches the Server
        let connect: Connect = Box::new(|_| Box::pin(async { Err(ErrorKind::Unsupported.into()) }));
        let shared = Arc::new(shared_for_tests(limits, writer, connect));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a free port");
        let address = listener.local_addr().expect("the listener has an address");

        // The first connection waits until its program closes, the second panics
        let served = tokio::spawn(serve_with(
            listener,
            shared,
            std::future::pending(),
            |mut program, _shared| async move {
                let mut byte = [0u8; 1];
                if program.read(&mut byte).await.is_ok_and(|read| read == 0) {
                    return;
                }
                panic!("a connection panics on purpose");
            },
        ));
        let waiting = TcpStream::connect(address).await.expect("connect");
        let mut panicking = TcpStream::connect(address).await.expect("connect");
        panicking.write_all(b"x").await.expect("send a byte");

        // The waiting connection is served on, as after a shutdown signal: no new connection
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), panicking.read_to_end(&mut rest))
            .await
            .expect("the panicking connection closes")
            .ok();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!served.is_finished(), "the other connection is waited for");
        assert!(
            TcpStream::connect(address).await.is_err(),
            "no new connection is accepted"
        );

        drop(waiting);
        let ended = tokio::time::timeout(Duration::from_secs(5), served)
            .await
            .expect("serve ends once the other connection is done")
            .expect("serve doesn't panic");
        assert_eq!(ended.0, Err(Stopped::ConnectionPanicked));
    }
}
