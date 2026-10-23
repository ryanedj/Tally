use proxy::{Limits, Stopped, create_log_file, serve, start_log, stop_log, wait_for_writer};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;
use tally_core::{
    check_settings, parse_secrets, parse_settings, read_config_file, settings_sha256,
};
use tokio::net::TcpListener;

/// 1. Proxy startup.
#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(settings_path), Some(secrets_path), None) = (args.next(), args.next(), args.next())
    else {
        eprintln!("usage: proxy <settings file> <secrets file>");
        return ExitCode::FAILURE;
    };
    let (settings_path, secrets_path) = (Path::new(&settings_path), Path::new(&secrets_path));

    // 1. Load the settings and the secrets
    let loaded = read_config_file(settings_path).and_then(|settings_bytes| {
        let raw = parse_settings(&settings_bytes, settings_path)?;
        let secrets = parse_secrets(&read_config_file(secrets_path)?, secrets_path)?;
        Ok((settings_bytes, raw, secrets))
    });
    let (settings_bytes, raw, secrets) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    // 2. Check them
    let settings = match check_settings(raw, &secrets) {
        Ok(settings) => settings,
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            return ExitCode::FAILURE;
        }
    };

    // 3. Bind the listen address, and listen for the shutdown signals, before the log exists
    let bound = TcpListener::bind(settings.listen_address())
        .await
        .and_then(|listener| {
            let address = listener.local_addr()?;
            Ok((listener, address))
        });
    let (listener, address) = match bound {
        Ok(bound) => bound,
        Err(error) => {
            eprintln!(
                "{}: the listen address can't be bound: {error}",
                settings.listen_address()
            );
            return ExitCode::FAILURE;
        }
    };
    // Registered now, so that a signal sent once "listening on" is printed can't stop the proxy
    // the default way
    #[cfg(unix)]
    let shutdown_signal = {
        use tokio::signal::unix::{SignalKind, signal};
        let listening = signal(SignalKind::interrupt())
            .and_then(|interrupt| Ok((interrupt, signal(SignalKind::terminate())?)));
        let (mut interrupt, mut terminate) = match listening {
            Ok(listening) => listening,
            Err(error) => {
                eprintln!("the shutdown signals can't be listened for: {error}");
                return ExitCode::FAILURE;
            }
        };
        async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        }
    };
    #[cfg(windows)]
    let shutdown_signal = {
        let mut ctrl_c = match tokio::signal::windows::ctrl_c() {
            Ok(ctrl_c) => ctrl_c,
            Err(error) => {
                eprintln!("the shutdown signals can't be listened for: {error}");
                return ExitCode::FAILURE;
            }
        };
        async move {
            ctrl_c.recv().await;
        }
    };

    // 4. Prepare the start line
    let mut run = [0u8; 16];
    if let Err(error) = getrandom::fill(&mut run) {
        eprintln!("no random run id: {error}");
        return ExitCode::FAILURE;
    }
    let run: String = run.iter().map(|byte| format!("{byte:02x}")).collect();
    let settings_sha256 = settings_sha256(&settings_bytes);

    // 5. Create this run's log file, start the Writer, log the start line
    let log_file = match create_log_file(settings.log_folder(), &run) {
        Ok(log_file) => log_file,
        Err(error) => {
            eprintln!(
                "{}: this run's log file can't be created: {error}",
                settings.log_folder().display()
            );
            return ExitCode::FAILURE;
        }
    };
    let log = match start_log(
        log_file,
        settings.client().to_string(),
        run,
        settings_sha256,
    )
    .await
    {
        Ok(log) => log,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    // 6. Serve, with DESIGN's time limits
    println!("listening on {address}");
    let limits = Limits {
        idle: Duration::from_secs(30),
        head: Duration::from_secs(10),
        body: Duration::from_secs(30),
        connect: Duration::from_secs(5),
        write: Duration::from_secs(30),
        server_silence: Duration::from_secs(30),
        shutdown: Duration::from_secs(5),
        gentle_close: Duration::from_secs(1),
        end_of_run: Duration::from_secs(4),
    };
    let (served, deadline) = serve(
        listener,
        settings,
        secrets,
        limits,
        log.writer.clone(),
        log.log_failed.clone(),
        shutdown_signal,
    )
    .await;

    // 7. Shut down: the stop line, after every other line, unless the run ends like a crash. All
    // of it by "deadline", the end-of-run limit after the cut-off
    if let Err(stopped) = served {
        // A failed log's error was printed by the Writer
        if stopped != Stopped::LogFailed {
            eprintln!("{stopped}");
        }
        // A log that didn't answer in time isn't waited for any longer
        if stopped != Stopped::LogTimedOut
            && let Err(error) = wait_for_writer(log, deadline).await
        {
            eprintln!("{error}");
        }
        return ExitCode::FAILURE;
    }
    if let Err(error) = stop_log(log, deadline).await {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
