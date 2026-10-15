mod connection;
mod endpoints;
mod error;
mod request;

pub use request::{Head, Received};

use crate::connection::handle;
use std::{io::ErrorKind, sync::Arc};
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    /// A complete request, head and body.
    Request(Received),
    /// Bytes that are not a complete request: a head httparse refuses,
    /// a request cut short by a close, or leftover bytes that don't parse.
    NotARequest(Vec<u8>),
}

/// What the fake API calls with each arrival, and the number of the connection it came on.
pub type Hook = Arc<dyn Fn(u64, &Arrival) + Send + Sync>;

/// The hook gets each arrival with the number of the connection it came on: 1 for the first
/// connection accepted, then 2, and so on. Only the first request on a connection is answered.
/// Runs until its task is dropped: an "accept" error is printed, and the next connection is
/// accepted.
pub async fn serve(listener: TcpListener, on_arrival: Hook) {
    let mut connection = 0;
    loop {
        let socket = match listener.accept().await {
            Ok((socket, _)) => socket,
            Err(e) => {
                eprintln!("accept error: {e}");
                if !matches!(
                    e.kind(),
                    ErrorKind::ConnectionRefused
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::ConnectionReset
                ) {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                continue;
            }
        };
        connection += 1;
        let hook = Arc::clone(&on_arrival);
        tokio::spawn(async move {
            if let Err(e) = handle(socket, connection, hook).await {
                eprintln!("connection {connection} error: {e}");
            }
        });
    }
}
