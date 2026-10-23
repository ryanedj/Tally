use super::handle_connection;
use crate::serve::{Connect, Limits, ServerStream, shared_for_tests};
use crate::writer::log_queue;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tally_core::{Entry, UpstreamReason};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const LIMITS: Limits = Limits {
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

/// A connection to the Server that takes "left" more bytes, then breaks, as one the Server
/// resets does. The Server never answers.
struct Breaking {
    left: usize,
}

impl AsyncRead for Breaking {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for Breaking {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.left == 0 {
            return Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()));
        }
        let written = bytes.len().min(self.left);
        self.left -= written;
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// A connection to the Server that never takes a byte, as one to a Server that stopped
/// reading. The Server never answers.
struct Stalled;

impl AsyncRead for Stalled {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for Stalled {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

/// Runs "handle_connection" on a loopback connection, with each connection to the Server made
/// by "server", and the test in the Writer's place. Sends a request carrying the key twice:
/// each gets its release line, then an upstream_error line with "reason", then "reply"; the
/// second shows the connection was kept open.
async fn assert_send_fails(
    server: fn() -> Box<dyn ServerStream>,
    limits: Limits,
    reason: UpstreamReason,
    reply: &[u8],
) {
    let (writer, mut queue) = log_queue(16);
    let connect: Connect = Box::new(move |_| Box::pin(async move { Ok(server()) }));
    let shared = Arc::new(shared_for_tests(limits, writer, connect));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let address = listener.local_addr().expect("the listener has an address");
    let mut program = TcpStream::connect(address).await.expect("connect");
    let (accepted, _) = listener.accept().await.expect("accept");
    tokio::spawn(async move { handle_connection(accepted, &shared).await });

    let request = b"GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n";
    for seq in [1, 3] {
        program.write_all(request).await.expect("send the request");
        let (release, answer_to) = timeout(Duration::from_secs(5), queue.recv())
            .await
            .expect("the release line within 5 s")
            .expect("the queue is open");
        assert!(
            matches!(&release, Entry::Release { keys, host }
                if keys.len() == 1 && keys.head == "weather_key" && host == "api.weather.local:80"),
            "{release:?}"
        );
        answer_to
            .send(seq)
            .expect("the connection waits for its answer");

        let (failed, answer_to) = timeout(Duration::from_secs(5), queue.recv())
            .await
            .expect("the upstream_error line within 5 s")
            .expect("the queue is open");
        let expected = Entry::UpstreamError {
            keys: vec!["weather_key".to_string()],
            host: "api.weather.local:80".to_string(),
            about: seq,
            reason,
        };
        assert_eq!(failed, expected);
        answer_to
            .send(seq + 1)
            .expect("the connection waits for its answer");

        let mut received = vec![0u8; reply.len()];
        timeout(Duration::from_secs(5), program.read_exact(&mut received))
            .await
            .expect("the reply within 5 s")
            .expect("read the reply");
        assert_eq!(
            String::from_utf8_lossy(&received),
            String::from_utf8_lossy(reply)
        );
    }
}

// Protocol 4's row "Sending to the Server fails". Over loopback the system takes a whole
// request, 1 MiB body included, before a Server can break the connection, so a stand-in
// breaks it part-way (DESIGN, Tests).
#[tokio::test]
async fn sending_to_the_server_fails() {
    assert_send_fails(
        || Box::new(Breaking { left: 16 }),
        LIMITS,
        UpstreamReason::SendFailed,
        b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n",
    )
    .await;
}

#[tokio::test]
async fn a_server_that_takes_no_byte_of_the_request() {
    let limits = Limits {
        server_silence: Duration::from_millis(100),
        ..LIMITS
    };
    assert_send_fails(
        || Box::new(Stalled),
        limits,
        UpstreamReason::ResponseTimeout,
        b"HTTP/1.1 504 Gateway Timeout\r\ncontent-length: 0\r\n\r\n",
    )
    .await;
}
