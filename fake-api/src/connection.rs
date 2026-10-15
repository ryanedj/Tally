use crate::endpoints::{Response, route};
use crate::error::HandleError;
use crate::request::{Parsed, parse_request};
use crate::{Arrival, Hook};
use tokio::{io::AsyncReadExt, io::AsyncWriteExt, net::TcpStream};

/// The most bytes read from a connection before the reply, and again after it. More than the
/// largest request the proxy forwards (an 8 KiB head and a 1 MiB body), so each one is
/// recorded whole, while a sender that never stops can't fill the memory.
const MAX_READ: usize = 2 * 1024 * 1024;

pub(crate) async fn handle(
    mut socket: TcpStream,
    connection: u64,
    hook: Hook,
) -> Result<(), HandleError> {
    let report = |arrival: &Arrival| hook(connection, arrival);
    let mut buf = Vec::new();
    let (first, used) = loop {
        let read = match socket.read_buf(&mut buf).await {
            Ok(read) => read,
            // A reset ends the connection like a close.
            Err(e) => {
                if !buf.is_empty() {
                    report(&Arrival::NotARequest(buf));
                }
                return Err(e.into());
            }
        };
        if read == 0 {
            if buf.is_empty() {
                return Ok(());
            }
            report(&Arrival::NotARequest(buf));
            return Err(HandleError::ClosedEarly);
        }
        match parse_request(&buf) {
            Parsed::Complete(received, used) => break (received, used),
            Parsed::Incomplete => {}
            Parsed::Malformed => {
                report(&Arrival::NotARequest(buf));
                // httparse can refuse a head before it is complete, so the rest of it,
                // and anything after, can still arrive.
                let after = reply(
                    &mut socket,
                    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
                if !after.is_empty() {
                    report(&Arrival::NotARequest(after));
                }
                return Err(HandleError::BadRequest);
            }
        }
        if buf.len() >= MAX_READ {
            report(&Arrival::NotARequest(buf));
            return Err(HandleError::TooLarge);
        }
    };
    let response = route(&first);
    report(&Arrival::Request(first));

    // For tests: a real server answers each request on a kept-alive connection, or discards
    // what follows once it closes. Here bytes after the first request are recorded
    // as further requests, in order, and never answered. A proxy that sends two requests on
    // one connection must show up in a test.
    let mut leftover = buf.split_off(used);
    record_leftover(&mut leftover, &report);
    let after = match response {
        Response::Bytes(bytes) => reply(&mut socket, &bytes).await,
        Response::Silent => read_until_close(&mut socket).await,
        Response::Endless => {
            // A failed write means the other side has closed: the normal end here.
            // Nothing is read meanwhile, so bytes sent during the stream aren't recorded.
            if socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await
                .is_ok()
            {
                let chunk = vec![b'x'; 64 * 1024];
                while socket.write_all(&chunk).await.is_ok() {}
            }
            Vec::new()
        }
        Response::Paused { first, then } => match socket.write_all(&first).await {
            Ok(()) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                reply(&mut socket, &then).await
            }
            Err(e) => {
                eprintln!("connection error during the reply: {e}");
                Vec::new()
            }
        },
    };
    leftover.extend_from_slice(&after);
    record_leftover(&mut leftover, &report);
    if !leftover.is_empty() {
        report(&Arrival::NotARequest(leftover));
    }
    Ok(())
}

/// Passes each complete request at the start of "leftover" to the hook and removes it.
/// Stops at the first thing that is not a complete request, and leaves it in "leftover".
fn record_leftover(leftover: &mut Vec<u8>, hook: &dyn Fn(&Arrival)) {
    while let Parsed::Complete(received, used) = parse_request(leftover) {
        hook(&Arrival::Request(received));
        *leftover = leftover.split_off(used);
    }
}

/// Sends "bytes", closes the write side only, then reads until the other side
/// closes (RFC 9112 §9.6). Returns what the other side sent in the meantime.
/// A failed write or shutdown ends the connection like a reset
async fn reply(socket: &mut TcpStream, bytes: &[u8]) -> Vec<u8> {
    if let Err(e) = socket.write_all(bytes).await {
        eprintln!("connection error during the reply: {e}");
        return Vec::new();
    }
    if let Err(e) = socket.shutdown().await {
        eprintln!("connection error during the reply: {e}");
        return Vec::new();
    }
    read_until_close(socket).await
}

/// Reads until the other side closes, or at least "MAX_READ" bytes have arrived.
/// For tests: a real server reads these bytes only to discard them, here they are kept and recorded.
async fn read_until_close(socket: &mut TcpStream) -> Vec<u8> {
    let mut after = Vec::new();
    loop {
        let read = match socket.read_buf(&mut after).await {
            Ok(read) => read,
            Err(e) => {
                eprintln!("connection error after the reply: {e}");
                return after;
            }
        };
        if read == 0 {
            return after;
        }
        if after.len() >= MAX_READ {
            eprintln!("stopped reading after {} bytes", after.len());
            return after;
        }
    }
}
