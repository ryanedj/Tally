use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(log_path), Some(settings_path), None) = (args.next(), args.next(), args.next())
    else {
        eprintln!("usage: checker <log file> <settings file>");
        return ExitCode::from(2);
    };
    ExitCode::from(checker::check(
        Path::new(&log_path),
        Path::new(&settings_path),
    ))
}
