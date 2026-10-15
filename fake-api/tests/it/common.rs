use fake_api::{Arrival, Received};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};

/// What the fake API has reported through its hook, in order, each with the number of the
/// connection it came on.
pub type Seen = Arc<Mutex<Vec<(u64, Arrival)>>>;

/// Starts a fake API on a free port and recording what reaches it.
pub async fn start() -> (SocketAddr, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let addr = listener.local_addr().expect("the listener has an address");
    let seen = Seen::default();
    let sink = Arc::clone(&seen);
    let hook = Arc::new(move |connection: u64, arrival: &Arrival| {
        sink.lock()
            .expect("the lock is not poisoned")
            .push((connection, arrival.clone()));
    });
    tokio::spawn(fake_api::serve(listener, hook));
    (addr, seen)
}

/// A request with no body.
pub fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").into_bytes()
}

pub async fn exchange(addr: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(request).await.expect("send the request");
    let mut reply = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut reply))
        .await
        .expect("the fake API closes within 5 s")
        .expect("read the reply");
    reply
}

/// What has arrived so far, without the connection numbers.
pub fn arrivals(seen: &Seen) -> Vec<Arrival> {
    seen.lock()
        .expect("the lock is not poisoned")
        .iter()
        .map(|(_, arrival)| arrival.clone())
        .collect()
}

/// The connection number of each complete request, in order.
pub fn request_connections(seen: &Seen) -> Vec<u64> {
    seen.lock()
        .expect("the lock is not poisoned")
        .iter()
        .filter_map(|(connection, arrival)| match arrival {
            Arrival::Request(_) => Some(*connection),
            _ => None,
        })
        .collect()
}

/// The complete requests among "arrivals", in order.
pub fn requests(arrivals: &[Arrival]) -> Vec<Received> {
    arrivals
        .iter()
        .filter_map(|arrival| match arrival {
            Arrival::Request(received) => Some(received.clone()),
            _ => None,
        })
        .collect()
}

pub async fn wait_until(seen: &Seen, done: impl Fn(&[Arrival]) -> bool) -> Vec<Arrival> {
    for _ in 0..200 {
        let now = arrivals(seen);
        if done(&now) {
            return now;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("still waiting after 2 s, arrivals: {:?}", arrivals(seen));
}
