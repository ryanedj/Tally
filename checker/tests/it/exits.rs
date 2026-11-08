// 6. Checker, one test per row of its exits table. Done when's four cases are
// "an_illegal_release", "a_deleted_line", "lines_cut_from_the_end" and
// "settings_that_dont_match_the_run".

use crate::common::{SETTINGS, check, good_run, log_of, run_checker, start_line};
use std::path::Path;

const NO_STOP: &str = "no stop line: a crash, a cut end, or a run still going";

/// The good run with line "index" replaced, once each "from" in it is replaced by its "to".
fn with_edit(index: usize, edits: &[(&str, &str)]) -> Vec<String> {
    let mut lines = good_run();
    let line = lines.get_mut(index).expect("the line exists");
    for (from, to) in edits {
        assert!(line.contains(from), "{from} isn't in {line}");
        *line = line.replace(from, to);
    }
    lines
}

/// Checks that "lines" get exit 1 and exactly "flags".
fn flagged(lines: &[String], flags: &[&str]) {
    let checked = check(log_of(lines), SETTINGS);
    assert_eq!(checked.flags(), flags, "{}", checked.printed);
    assert_eq!(checked.code, Some(1), "{}", checked.printed);
}

#[test]
fn a_clean_run() {
    let checked = check(log_of(&good_run()), SETTINGS);
    assert_eq!(checked.code, Some(0), "{}", checked.printed);
    assert!(checked.printed.contains("no flags"), "{}", checked.printed);
    assert!(
        checked
            .printed
            .contains("9 lines: start 1, release 2, pass 1, refuse 2, upstream_error 2, stop 1"),
        "{}",
        checked.printed
    );
}

// Malformed lines

#[test]
fn a_line_that_isnt_json() {
    let mut lines = good_run();
    lines.push("release weather_key".to_string());
    let checked = check(log_of(&lines), SETTINGS);
    assert!(
        checked.printed.contains("line 10: malformed line: "),
        "{}",
        checked.printed
    );
    assert_eq!(checked.code, Some(1));
}

#[test]
fn a_release_with_no_keys() {
    let lines = with_edit(6, &[("[\"maps_key\"]", "[]")]);
    let checked = check(log_of(&lines), SETTINGS);
    assert!(
        checked.printed.contains("line 7: malformed line: "),
        "{}",
        checked.printed
    );
    assert_eq!(checked.code, Some(1));
}

#[test]
fn a_pass_with_keys() {
    let lines = with_edit(2, &[("\"keys\":[]", "\"keys\":[\"weather_key\"]")]);
    let checked = check(log_of(&lines), SETTINGS);
    assert!(
        checked
            .printed
            .contains("line 3: malformed line: a pass line with keys"),
        "{}",
        checked.printed
    );
    assert_eq!(checked.code, Some(1));
}

// A malformed line's seq can't be read, so the next line also shows a gap
#[test]
fn a_malformed_line_in_the_middle_also_leaves_a_gap() {
    let mut lines = good_run();
    *lines.get_mut(4).expect("the line exists") = "not json".to_string();
    let checked = check(log_of(&lines), SETTINGS);
    let flags = checked.flags();
    assert_eq!(flags.len(), 2, "{flags:?}");
    assert!(
        flags
            .iter()
            .any(|flag| flag.starts_with("line 5: malformed"))
    );
    assert!(flags.contains(&"line 6 (seq 5): a seq gap, repeat or step back"));
}

// The first line

#[test]
fn the_first_line_isnt_a_start_line() {
    let lines = good_run().split_off(1);
    let checked = check(log_of(&lines), SETTINGS);
    assert_eq!(
        checked.flags(),
        ["the first line isn't a start line: not judged"]
    );
    assert_eq!(checked.code, Some(1));
    assert!(
        !checked.printed.contains("lines:"),
        "nothing else is judged"
    );
}

#[test]
fn an_empty_log_has_no_start_line() {
    let checked = check("", SETTINGS);
    assert_eq!(
        checked.flags(),
        ["the first line isn't a start line: not judged"]
    );
    assert_eq!(checked.code, Some(1));
}

#[test]
fn a_second_start_line() {
    let mut lines = good_run();
    let start = start_line(SETTINGS).replace("\"seq\":0", "\"seq\":4");
    *lines.get_mut(4).expect("the line exists") = start;
    flagged(&lines, &["line 5 (seq 4): a second start line"]);
}

// seq

#[test]
fn a_deleted_line() {
    let mut lines = good_run();
    lines.remove(4);
    flagged(&lines, &["line 5 (seq 5): a seq gap, repeat or step back"]);
}

#[test]
fn a_seq_repeat() {
    let lines = with_edit(4, &[("\"seq\":4", "\"seq\":3")]);
    flagged(
        &lines,
        &[
            "line 5 (seq 3): a seq gap, repeat or step back",
            "line 6 (seq 5): a seq gap, repeat or step back",
        ],
    );
}

#[test]
fn a_seq_step_back() {
    let lines = with_edit(4, &[("\"seq\":4", "\"seq\":1")]);
    flagged(
        &lines,
        &[
            "line 5 (seq 1): a seq gap, repeat or step back",
            "line 6 (seq 5): a seq gap, repeat or step back",
        ],
    );
}

#[test]
fn a_first_seq_that_isnt_0() {
    let mut lines = good_run();
    let line = lines.get_mut(0).expect("the start line");
    *line = line.replace("\"seq\":0", "\"seq\":1");
    flagged(
        &lines,
        &[
            "line 1 (seq 1): a seq gap, repeat or step back",
            "line 2 (seq 1): a seq gap, repeat or step back",
        ],
    );
}

// The settings

#[test]
fn a_wrong_client() {
    let lines = with_edit(3, &[("weather-app", "other-app")]);
    flagged(&lines, &["line 4 (seq 3): wrong client"]);
}

#[test]
fn an_illegal_release() {
    let lines = with_edit(6, &[("maps.local:8080", "api.weather.local:80")]);
    flagged(
        &lines,
        &["line 7 (seq 6): the key maps_key isn't allowed for api.weather.local:80"],
    );
}

// Every key is judged: the second one here is the illegal one
#[test]
fn a_release_with_two_keys_where_only_the_second_is_illegal() {
    let lines = with_edit(6, &[("[\"maps_key\"]", "[\"maps_key\",\"weather_key\"]")]);
    flagged(
        &lines,
        &["line 7 (seq 6): the key weather_key isn't allowed for maps.local:8080"],
    );
}

#[test]
fn a_release_of_a_key_the_settings_dont_name() {
    let lines = with_edit(6, &[("maps_key", "other_key")]);
    flagged(
        &lines,
        &["line 7 (seq 6): the key other_key isn't allowed for maps.local:8080"],
    );
}

#[test]
fn a_pass_to_a_host_that_isnt_listed() {
    let mut lines = good_run();
    let pass = r#"{"event":"pass","seq":4,"client":"weather-app","keys":[],"host":"evil.local:80","time":"2026-09-26T10:15:04Z"}"#;
    *lines.get_mut(4).expect("the line exists") = pass.to_string();
    flagged(
        &lines,
        &["line 5 (seq 4): a pass to evil.local:80, which isn't listed"],
    );
}

// upstream_error

const NO_DECISION: &str = "an upstream_error with no matching decision";

#[test]
fn an_upstream_error_about_a_line_that_isnt_a_decision() {
    let lines = with_edit(5, &[("\"about\":1", "\"about\":3")]);
    flagged(&lines, &[&format!("line 6 (seq 5): {NO_DECISION}")]);
}

#[test]
fn an_upstream_error_about_a_later_line() {
    let lines = with_edit(5, &[("\"about\":1", "\"about\":6")]);
    flagged(&lines, &[&format!("line 6 (seq 5): {NO_DECISION}")]);
}

#[test]
fn an_upstream_error_with_other_keys() {
    let lines = with_edit(5, &[("weather_key", "maps_key")]);
    flagged(&lines, &[&format!("line 6 (seq 5): {NO_DECISION}")]);
}

#[test]
fn an_upstream_error_with_another_host() {
    let lines = with_edit(5, &[("api.weather.local:80", "maps.local:8080")]);
    flagged(&lines, &[&format!("line 6 (seq 5): {NO_DECISION}")]);
}

// Keys are in name order, so a reorder only a hand edit can make is flagged
#[test]
fn an_upstream_error_with_its_keys_reordered() {
    let settings = SETTINGS.replace(
        "host = \"maps.local:8080\"",
        "host = \"api.weather.local:80\"",
    );
    let mut lines = good_run();
    *lines.get_mut(0).expect("the start line") = start_line(&settings);
    let line = lines.get_mut(1).expect("the release");
    *line = line.replace("[\"weather_key\"]", "[\"maps_key\",\"weather_key\"]");
    let line = lines.get_mut(5).expect("the upstream_error");
    *line = line.replace("[\"weather_key\"]", "[\"weather_key\",\"maps_key\"]");
    let line = lines.get_mut(6).expect("the second release");
    *line = line.replace("maps.local:8080", "api.weather.local:80");
    let checked = check(log_of(&lines), &settings);
    assert_eq!(checked.flags(), [format!("line 6 (seq 5): {NO_DECISION}")]);
}

// The end of the run

#[test]
fn lines_cut_from_the_end() {
    let mut lines = good_run();
    lines.truncate(6);
    flagged(&lines, &[NO_STOP]);
}

#[test]
fn a_line_after_the_stop_line() {
    let mut lines = good_run();
    let refuse = r#"{"event":"refuse","seq":9,"client":"weather-app","keys":[],"host":null,"reason":"malformed_request","time":"2026-09-26T10:15:09Z"}"#;
    lines.push(refuse.to_string());
    flagged(&lines, &["line 10 (seq 9): a line after the stop line"]);
}

#[test]
fn a_torn_last_line() {
    let mut lines = good_run();
    lines.truncate(8);
    let log = format!("{}{{\"event\":\"sto", log_of(&lines));
    let checked = check(log, SETTINGS);
    assert_eq!(checked.flags(), [NO_STOP], "torn, so not malformed");
    assert!(
        checked
            .printed
            .contains("line 9: a torn last line, not judged"),
        "{}",
        checked.printed
    );
    assert_eq!(checked.code, Some(1));
}

#[test]
fn a_torn_last_line_after_the_stop_line() {
    let log = format!("{}{{\"event\":\"rel", log_of(&good_run()));
    let checked = check(log, SETTINGS);
    assert_eq!(checked.flags(), ["line 10: a line after the stop line"]);
    assert_eq!(checked.code, Some(1));
}

// Not judged

#[test]
fn settings_that_dont_match_the_run() {
    let settings = SETTINGS.replace("weather-app", "other-app");
    let checked = check(log_of(&good_run()), &settings);
    assert_eq!(checked.code, Some(2), "{}", checked.printed);
    assert!(
        checked
            .printed
            .contains("not judged: the settings file doesn't match the run"),
        "{}",
        checked.printed
    );
    assert!(checked.flags().is_empty(), "{}", checked.printed);
}

#[test]
fn settings_that_dont_match_a_run_with_a_flag() {
    let settings = SETTINGS.replace("weather-app", "other-app");
    let mut lines = good_run();
    lines.push("not json".to_string());
    let checked = check(log_of(&lines), &settings);
    assert_eq!(checked.code, Some(1), "a flag wins over not judged");
    assert!(
        checked.printed.contains("not judged: "),
        "{}",
        checked.printed
    );
}

#[test]
fn settings_that_match_the_run_but_dont_parse() {
    let settings = "client = ";
    let mut lines = good_run();
    *lines.get_mut(0).expect("the start line") = start_line(settings);
    let checked = check(log_of(&lines), settings);
    assert_eq!(checked.code, Some(2), "{}", checked.printed);
    assert!(
        checked
            .printed
            .contains("not judged: the settings aren't valid: "),
        "{}",
        checked.printed
    );
}

#[test]
fn settings_that_match_the_run_but_break_a_rule() {
    let settings = SETTINGS.replace("TALLY_fedcba9876543210fedcba9876543210", "TALLY_bad");
    let mut lines = good_run();
    *lines.get_mut(0).expect("the start line") = start_line(&settings);
    let checked = check(log_of(&lines), &settings);
    assert_eq!(checked.code, Some(2), "{}", checked.printed);
    assert!(
        checked
            .printed
            .contains("not judged: the settings aren't valid: key maps_key: malformed placeholder"),
        "{}",
        checked.printed
    );
}

#[test]
fn a_log_that_cant_be_read() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let settings = folder.path().join("settings.toml");
    std::fs::write(&settings, SETTINGS).expect("the settings file");
    let checked = run_checker(&[&folder.path().join("missing.jsonl"), &settings]);
    assert_eq!(checked.code, Some(2));
    assert!(
        checked.printed.contains("missing.jsonl: can't be read"),
        "{}",
        checked.printed
    );
}

#[test]
fn a_settings_file_that_cant_be_read() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let log = folder.path().join("run.jsonl");
    std::fs::write(&log, log_of(&good_run())).expect("the log file");
    let checked = run_checker(&[&log, &folder.path().join("missing.toml")]);
    assert_eq!(checked.code, Some(2));
    assert!(
        checked.printed.contains("missing.toml: can't be read"),
        "{}",
        checked.printed
    );
}

#[test]
fn a_wrong_command_line() {
    for args in [vec![], vec![Path::new("a"), Path::new("b"), Path::new("c")]] {
        let checked = run_checker(&args);
        assert_eq!(checked.code, Some(2));
        assert!(checked.errors.contains("usage: "), "{}", checked.errors);
    }
}
