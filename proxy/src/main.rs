use proxy::{create_log_file, start_log, stop_log};
use std::path::Path;
use std::process::ExitCode;
use tally_core::{
    check_settings, parse_secrets, parse_settings, read_config_file, settings_sha256,
};

/// 1. Proxy startup. Steps 5 and 6, listening and shutting down, come at L5, for now the stop
/// line follows the start line.
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

    // 3. Prepare the start line
    let mut run = [0u8; 16];
    if let Err(error) = getrandom::fill(&mut run) {
        eprintln!("no random run id: {error}");
        return ExitCode::FAILURE;
    }
    let run: String = run.iter().map(|byte| format!("{byte:02x}")).collect();
    let settings_sha256 = settings_sha256(&settings_bytes);

    // 4. Create this run's log file, start the Writer, log the start line
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

    // 6. Shut down: the stop line, after every other line, then the Writer's end
    if let Err(error) = stop_log(log).await {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
