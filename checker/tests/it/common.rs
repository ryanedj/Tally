use std::path::Path;
use std::process::Command;
use tally_core::settings_sha256;

/// Two hosts and two keys: "weather_key" toward "api.weather.local:80", "maps_key" toward
/// "maps.local:8080".
pub const SETTINGS: &str = include_str!("../../../tally-core/tests/it/fixtures/two-keys.toml");

/// What the checker binary did.
pub struct Checked {
    pub code: Option<i32>,
    pub printed: String,
    pub errors: String,
}

impl Checked {
    /// The flags, in order.
    pub fn flags(&self) -> Vec<&str> {
        self.printed
            .lines()
            .skip_while(|line| *line != "flagged:")
            .skip(1)
            .take_while(|line| !line.starts_with("not judged"))
            .collect()
    }
}

/// Runs the checker binary with "args", as the auditor would.
pub fn run_checker(args: &[&Path]) -> Checked {
    let output = Command::new(env!("CARGO_BIN_EXE_checker"))
        .args(args)
        .output()
        .expect("the checker binary runs");
    Checked {
        code: output.status.code(),
        printed: String::from_utf8_lossy(&output.stdout).into_owned(),
        errors: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Checks "log" against "settings", each in a file of its own.
pub fn check(log: impl AsRef<[u8]>, settings: &str) -> Checked {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let log_path = folder.path().join("run.jsonl");
    let settings_path = folder.path().join("settings.toml");
    std::fs::write(&log_path, log).expect("the log file");
    std::fs::write(&settings_path, settings).expect("the settings file");
    run_checker(&[&log_path, &settings_path])
}

/// The start line of a run with these settings.
pub fn start_line(settings: &str) -> String {
    format!(
        r#"{{"event":"start","seq":0,"run":"8c1f0a3e9b7d4c62a5e01f9d3b7c4e18","client":"weather-app","settings_sha256":"{}","time":"2026-09-26T10:15:00Z"}}"#,
        settings_sha256(settings.as_bytes())
    )
}

/// A clean run with SETTINGS, each line without its "\n".
pub fn good_run() -> Vec<String> {
    let mut lines = vec![start_line(SETTINGS)];
    lines.extend(
        [
            r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:01Z"}"#,
            r#"{"event":"pass","seq":2,"client":"weather-app","keys":[],"host":"api.weather.local:80","time":"2026-09-26T10:15:02Z"}"#,
            r#"{"event":"refuse","seq":3,"client":"weather-app","keys":["weather_key"],"host":"evil.local:80","reason":"host_not_allowed","time":"2026-09-26T10:15:03Z"}"#,
            r#"{"event":"refuse","seq":4,"client":"weather-app","keys":[],"host":null,"reason":"malformed_request","time":"2026-09-26T10:15:04Z"}"#,
            r#"{"event":"upstream_error","seq":5,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","about":1,"reason":"response_timeout","time":"2026-09-26T10:15:05Z"}"#,
            r#"{"event":"release","seq":6,"client":"weather-app","keys":["maps_key"],"host":"maps.local:8080","time":"2026-09-26T10:15:06Z"}"#,
            r#"{"event":"upstream_error","seq":7,"client":"weather-app","keys":[],"host":"api.weather.local:80","about":2,"reason":"connect_failed","time":"2026-09-26T10:15:07Z"}"#,
            r#"{"event":"stop","seq":8,"client":"weather-app","time":"2026-09-26T10:15:08Z"}"#,
        ]
        .map(String::from),
    );
    lines
}

/// "lines" as a log file, each ending in "\n".
pub fn log_of(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}
