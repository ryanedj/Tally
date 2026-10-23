use crate::serve::Shared;
use crate::wait::{WriteFailed, until, write_with_progress};
use crate::writer::LogFailed;
use tally_core::{Entry, Refusal, RefuseReason, UpstreamReason};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;

/// Whether the program's connection stays open after a request (see 5).
pub(crate) enum Outcome {
    KeepOpen,
    /// Closed gently: after a complete reply, or a response cut short.
    Close,
    /// Closed at once: the log failed, a write to the program gave up, or the cut-off came.
    Stop,
}

/// A request whose release or pass line is on disk.
pub(crate) struct Forwarded {
    pub(crate) keys: Vec<String>,
    pub(crate) host: String,
    /// The release or pass line's.
    pub(crate) seq: u64,
    /// The program sent "Connection: close".
    pub(crate) wants_close: bool,
}

/// Hands "entry" to the Writer and waits until its line is on disk (calls 2): its seq. "None"
/// when the log failed: the proxy stops, and so does this connection, after a 503 if "can_reply".
pub(crate) async fn log_or_stop(
    program: &mut TcpStream,
    entry: Entry,
    can_reply: bool,
    shared: &Shared,
) -> Option<u64> {
    match shared.writer.submit(entry).await {
        Ok(seq) => Some(seq),
        Err(LogFailed) => {
            // While the proxy serves, an Error answer means the log failed. The Writer sets
            // log_failed after answering, so a shutdown could otherwise end as if it hadn't.
            shared.log_failed.cancel();
            if can_reply {
                // Tried once, never waited for, the cut-off comes with the failure, so the
                // write is polled first, and goes out if the socket takes it
                let reply = b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
                tokio::select! {
                    biased;
                    _ = program.write_all(reply) => {}
                    () = shared.cut_off.cancelled() => {}
                }
            }
            None
        }
    }
}

/// Step 3, refuse: the refuse line, then the reply and a gentle close, unless the program has
/// closed its side.
pub(crate) async fn refuse(
    program: &mut TcpStream,
    refusal: Refusal,
    can_reply: bool,
    shared: &Shared,
) {
    let reason = refusal.reason;
    let entry = Entry::Refuse {
        keys: refusal.keys,
        host: refusal.host,
        reason,
    };
    if log_or_stop(program, entry, can_reply, shared)
        .await
        .is_none()
        || !can_reply
    {
        return;
    }
    let (code, phrase) = match reason {
        RefuseReason::IncompleteRequest => (408, "Request Timeout"),
        RefuseReason::HeadTooLarge => (431, "Request Header Fields Too Large"),
        RefuseReason::BodyTooLarge => (413, "Content Too Large"),
        RefuseReason::MalformedRequest => (400, "Bad Request"),
        RefuseReason::UnsupportedRequest => (501, "Not Implemented"),
        RefuseReason::HostNotAllowed => (403, "Forbidden"),
    };
    // The reason's name, as the log writes it
    let name = serde_json::to_string(&reason).unwrap_or_default();
    let body = name.trim_matches('"').as_bytes();
    if reply(program, code, phrase, true, body, shared)
        .await
        .is_ok()
    {
        close_gently(program, shared).await;
    }
}

/// The upstream_error line after a release or pass (calls 2), then the reply its reason has, if
/// any.
pub(crate) async fn upstream_failed(
    program: &mut TcpStream,
    forwarded: &Forwarded,
    reason: UpstreamReason,
    shared: &Shared,
) -> Outcome {
    let entry = Entry::UpstreamError {
        keys: forwarded.keys.clone(),
        host: forwarded.host.clone(),
        about: forwarded.seq,
        reason,
    };
    // Once the final head was relayed, the response just stops
    let can_reply = reason != UpstreamReason::ResponseIncomplete;
    if log_or_stop(program, entry, can_reply, shared)
        .await
        .is_none()
    {
        return Outcome::Stop;
    }
    let closing = matches!(
        reason,
        UpstreamReason::BadResponse | UpstreamReason::ResponseIncomplete | UpstreamReason::Shutdown
    ) || forwarded.wants_close
        || shared.shutdown.is_cancelled();
    let status = match reason {
        UpstreamReason::ConnectFailed
        | UpstreamReason::SendFailed
        | UpstreamReason::BadResponse => Some((502, "Bad Gateway")),
        UpstreamReason::ConnectTimeout | UpstreamReason::ResponseTimeout => {
            Some((504, "Gateway Timeout"))
        }
        UpstreamReason::Shutdown | UpstreamReason::ResponseIncomplete => None,
    };
    if let Some((code, phrase)) = status
        && reply(program, code, phrase, closing, b"", shared)
            .await
            .is_err()
    {
        return Outcome::Stop;
    }
    if closing {
        Outcome::Close
    } else {
        Outcome::KeepOpen
    }
}

/// Every reply the proxy writes itself, with "connection: close" when the connection closes after
/// it.
async fn reply(
    program: &mut TcpStream,
    code: u16,
    phrase: &str,
    closing: bool,
    body: &[u8],
    shared: &Shared,
) -> Result<(), WriteFailed> {
    let mut head = format!(
        "HTTP/1.1 {code} {phrase}\r\ncontent-length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        head.push_str("content-type: text/plain\r\n");
    }
    if closing {
        head.push_str("connection: close\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    write_with_progress(program, &bytes, shared.limits.write, &shared.cut_off).await
}

/// Stops writing, then reads and discards what the program still sends, for at most the
/// gentle-close limit or 1 MiB, before the close (RFC 9112 §9.6). Ends at once after the cut-off.
pub(crate) async fn close_gently(program: &mut TcpStream, shared: &Shared) {
    if program.shutdown().await.is_err() {
        return;
    }
    let deadline = Instant::now() + shared.limits.gentle_close;
    let mut discarded = 0;
    let mut scratch = [0u8; 16 * 1024];
    while discarded < 1024 * 1024 {
        match until(program.read(&mut scratch), deadline, &shared.cut_off).await {
            Ok(Ok(read)) if read > 0 => discarded += read,
            _ => return,
        }
    }
}
