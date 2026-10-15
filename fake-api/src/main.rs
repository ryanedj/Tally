use fake_api::Arrival;
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let address = std::env::args().nth(1).expect("usage: fake-api <address>");
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("listening on {}", listener.local_addr()?);
    let hook = Arc::new(|connection: u64, arrival: &Arrival| match arrival {
        Arrival::Request(r) => println!(
            "--- connection {connection}: request received {} ---",
            String::from_utf8_lossy(&r.raw)
        ),
        Arrival::NotARequest(bytes) => println!(
            "--- connection {connection}: not a request {} ---",
            String::from_utf8_lossy(bytes)
        ),
    });
    fake_api::serve(listener, hook).await;
    Ok(())
}
