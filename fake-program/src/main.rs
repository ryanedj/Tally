use fake_program::Placeholders;
use std::env::VarError;
use std::net::SocketAddr;
use std::process::ExitCode;

/// The sandboxed weather app of the demo. It reads its configuration from its environment, as an
/// app would: the proxy in "http_proxy", its placeholders where it would read its API keys.
#[tokio::main]
async fn main() -> ExitCode {
    let var = |name: &str| match std::env::var(name) {
        Ok(value) => Some(value),
        Err(VarError::NotPresent) => {
            eprintln!("{name} isn't set");
            None
        }
        Err(VarError::NotUnicode(_)) => {
            eprintln!("{name} isn't valid Unicode");
            None
        }
    };
    let (Some(proxy), Some(weather), Some(maps)) =
        (var("http_proxy"), var("WEATHER_KEY"), var("MAPS_KEY"))
    else {
        return ExitCode::FAILURE;
    };

    let address = proxy.strip_prefix("http://").unwrap_or(&proxy);
    let Ok(address) = address
        .strip_suffix('/')
        .unwrap_or(address)
        .parse::<SocketAddr>()
    else {
        eprintln!(
            "http_proxy isn't an IP address and port, such as http://127.0.0.1:8080: {proxy}"
        );
        return ExitCode::FAILURE;
    };

    if let Err(error) = fake_program::run(address, &Placeholders { weather, maps }).await {
        eprintln!("the proxy at {address} can't be reached: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
