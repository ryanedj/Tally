use crate::relay::relay_response;
use crate::reply::{Forwarded, Outcome, close_gently, log_or_stop, refuse, upstream_failed};
use crate::serve::Shared;
use crate::wait::{Interrupted, WriteFailed, read_more, until, write_with_progress};
use tally_core::{
    Decision, Entry, Method, Parsed, Refusal, RefuseReason, UpstreamReason, decide,
    parse_request_head,
};
use tokio::net::TcpStream;
use tokio::time::Instant;

/// 4. One program connection. The connection closes once "program" is dropped.
pub(crate) async fn handle_connection(mut program: TcpStream, shared: &Shared) {
    let limits = &shared.limits;
    // Bytes read from the program but not handled yet
    let mut buffer = Vec::new();
    // A refusal whose keys and host aren't known yet
    let unknown = |reason| Refusal {
        reason,
        keys: Vec::new(),
        host: None,
    };

    loop {
        // 1. Read the head and check it follows the strict HTTP rules
        if buffer.is_empty() {
            // Before the first byte, a close, silence, the shutdown or the cut-off ends the
            // connection with no line and no reply
            let read = tokio::select! {
                biased;
                () = shared.shutdown.cancelled() => return,
                read = read_more(&mut program, &mut buffer, Instant::now() + limits.idle, &shared.cut_off) => read,
            };
            if !matches!(read, Ok(Ok(read)) if read > 0) {
                return;
            }
        }
        let deadline = Instant::now() + limits.head;
        let request = loop {
            match parse_request_head(&buffer) {
                Ok(Parsed::Complete(request, length)) => {
                    buffer.drain(..length);
                    break request;
                }
                Ok(Parsed::Partial) => {}
                // A line, or the whole head, breaks a rule
                Err(error) => {
                    return refuse(&mut program, unknown(error.into()), true, shared).await;
                }
            }
            let can_reply =
                match read_more(&mut program, &mut buffer, deadline, &shared.cut_off).await {
                    Ok(Ok(0) | Err(_)) => false,
                    Ok(Ok(_)) => continue,
                    Err(Interrupted::TimedOut) => true,
                    Err(Interrupted::CutOff) => return,
                };
            let refusal = unknown(RefuseReason::IncompleteRequest);
            return refuse(&mut program, refusal, can_reply, shared).await;
        };

        // 2. Decide the request: the first case that applies (calls 3)
        let (entry, keys, host) = match decide(&request, &shared.settings) {
            // 3. Refuse
            Decision::Refuse(refusal) => return refuse(&mut program, refusal, true, shared).await,
            Decision::Release { keys, host } => (
                Entry::Release {
                    keys: keys.clone(),
                    host: host.clone(),
                },
                Vec::from(keys),
                host,
            ),
            Decision::Pass { host } => (Entry::Pass { host: host.clone() }, Vec::new(), host),
        };

        // 4. Read the rest of the body, then log the decision
        // The parser allows at most 1 MiB, so the length fits
        let length = usize::try_from(request.content_length).unwrap_or(usize::MAX);
        let deadline = Instant::now() + limits.body;
        while buffer.len() < length {
            let can_reply =
                match read_more(&mut program, &mut buffer, deadline, &shared.cut_off).await {
                    Ok(Ok(0) | Err(_)) => false,
                    Ok(Ok(_)) => continue,
                    Err(Interrupted::TimedOut) => true,
                    Err(Interrupted::CutOff) => return,
                };
            let refusal = Refusal {
                reason: RefuseReason::IncompleteRequest,
                keys,
                host: Some(host),
            };
            return refuse(&mut program, refusal, can_reply, shared).await;
        }
        let body: Vec<u8> = buffer.drain(..length).collect();
        let Some(seq) = log_or_stop(&mut program, entry, true, shared).await else {
            return;
        };
        let forwarded = Forwarded {
            keys,
            host,
            seq,
            wants_close: request.wants_close,
        };

        // 5. Rewrite the request and swap each placeholder for its real key
        let method = match request.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        };
        // RFC 3986 §3.2.3: the default port is left out
        let host_value = if request.port == 80 {
            request.host.clone()
        } else {
            format!("{}:{}", request.host, request.port)
        };
        let mut upstream = format!(
            "{method} {} HTTP/1.1\r\nhost: {host_value}\r\n",
            request.path_and_query
        )
        .into_bytes();
        for (name, value) in &request.headers {
            if matches!(name.as_str(), "host" | "connection" | "keep-alive" | "te")
                || name.starts_with("proxy-")
            {
                continue;
            }
            let mut value = value.clone();
            for key_name in &forwarded.keys {
                let (Some(key), Some(secret)) = (
                    shared.settings.keys().get(key_name),
                    shared.secrets.get(key_name),
                ) else {
                    continue;
                };
                if key.header() != name.as_str() {
                    continue;
                }
                // Every occurrence, in its bound header only
                let mut swapped = Vec::with_capacity(value.len());
                let mut rest = value.as_slice();
                loop {
                    if let Some(tail) = rest.strip_prefix(key.placeholder().as_bytes()) {
                        swapped.extend_from_slice(secret.as_bytes());
                        rest = tail;
                    } else if let [first, tail @ ..] = rest {
                        swapped.push(*first);
                        rest = tail;
                    } else {
                        break;
                    }
                }
                value = swapped;
            }
            upstream.extend_from_slice(name.as_bytes());
            upstream.extend_from_slice(b": ");
            upstream.extend_from_slice(&value);
            upstream.extend_from_slice(b"\r\n");
        }
        upstream.extend_from_slice(b"connection: close\r\n\r\n");
        upstream.extend_from_slice(&body);

        let sent = async {
            // From the settings, never from DNS.
            let address = shared
                .settings
                .hosts()
                .get(&forwarded.host)
                .copied()
                .ok_or(UpstreamReason::ConnectFailed)?;
            let connected = until(
                (shared.connect)(address),
                Instant::now() + limits.connect,
                &shared.cut_off,
            )
            .await;
            let mut server = match connected {
                Ok(Ok(server)) => server,
                Ok(Err(_)) => return Err(UpstreamReason::ConnectFailed),
                Err(Interrupted::TimedOut) => return Err(UpstreamReason::ConnectTimeout),
                Err(Interrupted::CutOff) => return Err(UpstreamReason::Shutdown),
            };
            match write_with_progress(
                &mut server,
                &upstream,
                limits.server_silence,
                &shared.cut_off,
            )
            .await
            {
                Ok(()) => Ok(server),
                Err(WriteFailed::Broken) => Err(UpstreamReason::SendFailed),
                Err(WriteFailed::TimedOut) => Err(UpstreamReason::ResponseTimeout),
                Err(WriteFailed::CutOff) => Err(UpstreamReason::Shutdown),
            }
        }
        .await;
        let outcome = match sent {
            // 6. Relay the response (calls 5). One Server connection per request: it closes once
            // dropped.
            Ok(mut server) => relay_response(&mut server, &mut program, &forwarded, shared).await,
            Err(reason) => upstream_failed(&mut program, &forwarded, reason, shared).await,
        };

        // 7. Keep the connection for the next request
        match outcome {
            Outcome::KeepOpen => {}
            // Bytes left in "buffer" are never handled
            Outcome::Close => return close_gently(&mut program, shared).await,
            Outcome::Stop => return,
        }
    }
}

#[cfg(test)]
mod tests;
