// What the report prints, and logs an edit could use against the checker.

use crate::common::{SETTINGS, check, good_run, log_of};

#[test]
fn the_report_gives_rows_then_counts_then_flags() {
    let checked = check(log_of(&good_run()), SETTINGS);
    let expected = "\
seq event          keys        host                 reason            about
  0 start
  1 release        weather_key api.weather.local:80
  2 pass                       api.weather.local:80
  3 refuse         weather_key evil.local:80        host_not_allowed
  4 refuse                                          malformed_request
  5 upstream_error weather_key api.weather.local:80 response_timeout  1
  6 release        maps_key    maps.local:8080
  7 upstream_error             api.weather.local:80 connect_failed    2
  8 stop

9 lines: start 1, release 2, pass 1, refuse 2, upstream_error 2, stop 1
refuse: host_not_allowed 1, malformed_request 1
upstream_error: response_timeout 1, connect_failed 1

no flags
";
    assert_eq!(checked.printed, expected);
}

// Rust ignores SIGPIPE, so a reader that closes early is a write error: exit 2, the
// report has to outgrow the pipe's buffer.
#[test]
fn a_reader_that_closes_early_is_a_write_error_not_a_panic() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let mut lines = good_run();
    let stop = lines.pop().expect("the stop line");
    let pass = r#"{"event":"pass","seq":SEQ,"client":"weather-app","keys":[],"host":"api.weather.local:80","time":"2026-09-26T10:15:02Z"}"#;
    for seq in 8..30_000 {
        lines.push(pass.replace("SEQ", &seq.to_string()));
    }
    lines.push(stop.replace("\"seq\":8", "\"seq\":30000"));
    let log = folder.path().join("run.jsonl");
    let settings = folder.path().join("settings.toml");
    std::fs::write(&log, log_of(&lines)).expect("the log file");
    std::fs::write(&settings, SETTINGS).expect("the settings file");

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_checker"))
        .args([&log, &settings])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the checker binary runs");
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("the checker ends");
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{errors}");
    assert!(errors.contains("write error"), "{errors}");
    assert!(!errors.contains("panicked"), "{errors}");
}

#[test]
fn a_byte_that_isnt_utf8_spoils_one_line() {
    let mut log = log_of(&good_run()).into_bytes();
    log.extend_from_slice(b"\xff\n");
    let checked = check(log, SETTINGS);
    assert_eq!(checked.code, Some(1), "flagged, not an unreadable file");
    assert_eq!(checked.flags(), ["line 10: malformed line: not UTF-8"]);
}

#[test]
fn the_largest_seq_doesnt_overflow() {
    let mut lines = good_run();
    let line = lines.get_mut(4).expect("the line exists");
    *line = line.replace("\"seq\":4", "\"seq\":18446744073709551615");
    let checked = check(log_of(&lines), SETTINGS);
    assert_eq!(checked.code, Some(1), "{}", checked.errors);
    assert_eq!(
        checked.flags(),
        [
            "line 5 (seq 18446744073709551615): a seq gap, repeat or step back",
            "line 6 (seq 5): a seq gap, repeat or step back",
        ]
    );
}

// No rule judges a refusal's host, so only the printing keeps it on its row
#[test]
fn a_newline_in_a_host_cant_forge_a_line_of_the_report() {
    let mut lines = good_run();
    let line = lines.get_mut(3).expect("the refusal");
    *line = line.replace("evil.local:80", "evil.local:80\\n\\nno flags");
    let checked = check(log_of(&lines), SETTINGS);
    assert_eq!(checked.code, Some(0));
    assert_eq!(
        checked
            .printed
            .lines()
            .filter(|line| *line == "no flags")
            .count(),
        1
    );
    assert!(checked.printed.contains("evil.local:80\\n\\nno flags"));
}

// Its fingerprint can't match, so toml never reads it and can't quote a value
#[test]
fn a_secrets_file_given_as_the_settings_is_never_parsed() {
    let secrets = "hosts = \"the-real-key\"\n";
    let checked = check(log_of(&good_run()), secrets);
    assert_eq!(checked.code, Some(2));
    assert!(
        !checked.printed.contains("the-real-key"),
        "{}",
        checked.printed
    );
}

#[test]
fn a_run_that_isnt_judged_gets_no_rows() {
    let settings = SETTINGS.replace("weather-app", "other-app");
    let checked = check(log_of(&good_run()), &settings);
    assert!(!checked.printed.contains("lines:"), "{}", checked.printed);
    assert!(!checked.printed.contains("release"), "{}", checked.printed);
}
