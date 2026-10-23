use crate::reply::{Forwarded, Outcome, upstream_failed};
use crate::serve::Shared;
use crate::wait::{Interrupted, read_more, write_with_progress};
use tally_core::{BodyEnd, Parsed, UpstreamReason, parse_response_head};
use tokio::io::AsyncRead;
use tokio::net::TcpStream;
use tokio::time::Instant;

/// 5. Relay response, from "server", which has just been sent the request.
pub(crate) async fn relay_response(
    server: &mut (impl AsyncRead + Unpin),
    program: &mut TcpStream,
    forwarded: &Forwarded,
    shared: &Shared,
) -> Outcome {
    let limits = &shared.limits;
    // Each wait for the Server gives up after this much silence
    let silence = || Instant::now() + limits.server_silence;
    // Bytes read from the Server but not relayed yet
    let mut buffer = Vec::new();

    // Once per head: any interim (1xx) heads, then the final one
    let (body_end, closing) = loop {
        // 1. Wait for the response head
        let response = loop {
            match parse_response_head(&buffer) {
                Ok(Parsed::Complete(response, length)) => {
                    buffer.drain(..length);
                    break Some(response);
                }
                Ok(Parsed::Partial) => {}
                // 2. Check the head: a line, or the whole head, breaks a rule (see DESIGN,
                // Responses)
                Err(_) => break None,
            }
            match read_more(server, &mut buffer, silence(), &shared.cut_off).await {
                // The Server closed, or broke the connection, before the end of the head
                Ok(Ok(0) | Err(_)) => break None,
                Ok(Ok(_)) => {}
                Err(Interrupted::TimedOut) => {
                    let reason = UpstreamReason::ResponseTimeout;
                    return upstream_failed(program, forwarded, reason, shared).await;
                }
                Err(Interrupted::CutOff) => return Outcome::Stop,
            }
        };

        // 3. Bad response
        let Some(response) = response else {
            let reason = UpstreamReason::BadResponse;
            return upstream_failed(program, forwarded, reason, shared).await;
        };

        // 4. Find where the response ends, rewrite the head
        let is_final = !(100..200).contains(&response.status);
        let closing = is_final
            && (response.body_end == BodyEnd::UntilServerCloses
                || forwarded.wants_close
                || shared.shutdown.is_cancelled());
        let mut head = format!("HTTP/1.1 {} ", response.status).into_bytes();
        head.extend_from_slice(&response.reason);
        head.extend_from_slice(b"\r\n");
        // content-length and transfer-encoding stay: the body is relayed unchanged
        for (name, value) in &response.headers {
            let is_dropped = name == "connection"
                || name == "keep-alive"
                || name.starts_with("proxy-")
                || response.connection_names.contains(name);
            if !is_dropped {
                head.extend_from_slice(name.as_bytes());
                head.extend_from_slice(b": ");
                head.extend_from_slice(value);
                head.extend_from_slice(b"\r\n");
            }
        }
        if closing {
            head.extend_from_slice(b"connection: close\r\n");
        }
        head.extend_from_slice(b"\r\n");
        if write_with_progress(program, &head, limits.write, &shared.cut_off)
            .await
            .is_err()
        {
            return Outcome::Stop;
        }
        if is_final {
            break (response.body_end, closing);
        }
    };

    // 5. Relay the body, what's left in "buffer" is its start
    let is_complete = match body_end {
        BodyEnd::NoBody => true,
        BodyEnd::Length(mut left) => loop {
            // Bytes past the announced length are never relayed
            let take = usize::try_from(left).map_or(buffer.len(), |left| left.min(buffer.len()));
            let part = buffer.get(..take).unwrap_or_default();
            if write_with_progress(program, part, limits.write, &shared.cut_off)
                .await
                .is_err()
            {
                return Outcome::Stop;
            }
            left -= take as u64;
            if left == 0 {
                break true;
            }
            buffer.clear();
            match read_more(server, &mut buffer, silence(), &shared.cut_off).await {
                Ok(Ok(0) | Err(_)) | Err(Interrupted::TimedOut) => break false,
                Ok(Ok(_)) => {}
                Err(Interrupted::CutOff) => return Outcome::Stop,
            }
        },
        BodyEnd::UntilServerCloses => loop {
            if write_with_progress(program, &buffer, limits.write, &shared.cut_off)
                .await
                .is_err()
            {
                return Outcome::Stop;
            }
            buffer.clear();
            match read_more(server, &mut buffer, silence(), &shared.cut_off).await {
                Ok(Ok(0)) => break true,
                Ok(Ok(_)) => {}
                // RFC 9112 §8: complete "unless an error was indicated by the underlying
                // connection"
                Ok(Err(_)) | Err(Interrupted::TimedOut) => break false,
                Err(Interrupted::CutOff) => return Outcome::Stop,
            }
        },
    };
    if !is_complete {
        let reason = UpstreamReason::ResponseIncomplete;
        return upstream_failed(program, forwarded, reason, shared).await;
    }

    // 6. Return to 4
    if closing {
        Outcome::Close
    } else {
        Outcome::KeepOpen
    }
}
