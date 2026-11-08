use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use tally_core::{
    LogLine, check_settings, parse_log_line, parse_settings, read_config_file, settings_sha256,
};

/// Adds one to "name" in "counts", which keeps the order names first appear in.
fn count(counts: &mut Vec<(String, u64)>, name: &str) {
    match counts.iter_mut().find(|(counted, _)| counted == name) {
        Some((_, total)) => *total += 1,
        None => counts.push((name.to_string(), 1)),
    }
}

/// "start 1, stop 1".
fn joined(counts: &[(String, u64)]) -> String {
    let parts: Vec<String> = counts
        .iter()
        .map(|(name, total)| format!("{name} {total}"))
        .collect();
    parts.join(", ")
}

/// 6. Checker: prints the report on a run's log, and returns the exit code.
pub fn check(log_path: &Path, settings_path: &Path) -> u8 {
    // 1. Fingerprint the settings file
    let settings_bytes = match read_config_file(settings_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            // The exit code is 2 whether or not the line is written
            let _ = writeln!(std::io::stdout(), "not judged: {error}");
            return 2;
        }
    };
    let fingerprint = settings_sha256(&settings_bytes);

    // 2. Read the run's log
    let log = match std::fs::read(log_path) {
        Ok(log) => log,
        Err(error) => {
            let _ = writeln!(
                std::io::stdout(),
                "not judged: {}: can't be read: {error}",
                log_path.display()
            );
            return 2;
        }
    };
    let mut flags = Vec::new();
    let mut run_lines = Vec::new();
    let mut torn_line = None;
    for (number, part) in (1u64..).zip(log.split_inclusive(|&byte| byte == b'\n')) {
        let Some(text) = part.strip_suffix(b"\n") else {
            torn_line = Some(number);
            continue;
        };
        let line = match std::str::from_utf8(text) {
            Ok(text) => parse_log_line(text).map_err(|error| error.to_string()),
            Err(_) => Err("not UTF-8".to_string()),
        };
        match line {
            Ok(line) => run_lines.push((number, line)),
            Err(error) => flags.push(format!(
                "line {number}: malformed line: {}",
                error.escape_debug()
            )),
        }
    }

    // 3. Not judged: no start line, different settings, or settings that aren't valid
    let mut not_judged = None;
    let settings = match run_lines.first() {
        Some((
            _,
            LogLine::Start {
                settings_sha256, ..
            },
        )) if *settings_sha256 == fingerprint => {
            let checked = parse_settings(&settings_bytes, settings_path)
                .map_err(|error| error.to_string())
                .and_then(|raw| {
                    check_settings(raw).map_err(|problems| {
                        let problems: Vec<String> =
                            problems.iter().map(ToString::to_string).collect();
                        problems.join("; ")
                    })
                });
            match checked {
                Ok(settings) => Some(settings),
                Err(error) => {
                    not_judged = Some(format!("the settings aren't valid: {error}"));
                    None
                }
            }
        }
        Some((_, LogLine::Start { .. })) => {
            not_judged = Some("the settings file doesn't match the run".to_string());
            None
        }
        _ => {
            flags.push("the first line isn't a start line: not judged".to_string());
            None
        }
    };

    // 4. Check every line
    let mut rows = Vec::new();
    let mut events = Vec::new();
    let mut refusals = Vec::new();
    let mut upstream_errors = Vec::new();
    if let Some(settings) = &settings {
        let mut decisions = BTreeMap::new();
        let mut previous_seq = None;
        let mut stop_seen = false;
        for (position, (number, line)) in run_lines.iter().enumerate() {
            let (LogLine::Start { seq, client, .. }
            | LogLine::Release { seq, client, .. }
            | LogLine::Pass { seq, client, .. }
            | LogLine::Refuse { seq, client, .. }
            | LogLine::UpstreamError { seq, client, .. }
            | LogLine::Stop { seq, client, .. }) = line;
            let mut flag = |what: &str| flags.push(format!("line {number} (seq {seq}): {what}"));
            // An edited log can hold a newline, which could forge a line of this report
            let shown_host = |host: &str| host.escape_debug().to_string();

            if stop_seen {
                flag("a line after the stop line");
            }
            let expected_seq = match previous_seq {
                None => Some(0),
                Some(previous) => u64::checked_add(previous, 1),
            };
            if Some(*seq) != expected_seq {
                flag("a seq gap, repeat or step back");
            }
            previous_seq = Some(*seq);
            if client != settings.client() {
                flag("wrong client");
            }

            let mut about_cell = String::new();
            let (event, keys, host, reason) = match line {
                LogLine::Start { .. } => {
                    if position != 0 {
                        flag("a second start line");
                    }
                    ("start", Vec::new(), "", String::new())
                }
                LogLine::Release { keys, host, .. } => {
                    let names = Vec::from(keys.clone());
                    for name in &names {
                        let key = settings.keys().get(name);
                        if key.is_none_or(|key| key.host() != host) {
                            flag(&format!(
                                "the key {} isn't allowed for {}",
                                name.escape_debug(),
                                shown_host(host)
                            ));
                        }
                    }
                    decisions.insert(*seq, (names.clone(), host));
                    ("release", names, host.as_str(), String::new())
                }
                LogLine::Pass { keys, host, .. } => {
                    if !settings.hosts().contains_key(host) {
                        flag(&format!(
                            "a pass to {}, which isn't listed",
                            shown_host(host)
                        ));
                    }
                    decisions.insert(*seq, (keys.clone(), host));
                    ("pass", keys.clone(), host.as_str(), String::new())
                }
                LogLine::Refuse {
                    keys, host, reason, ..
                } => {
                    let reason = serde_json::to_string(reason).unwrap_or_default();
                    let reason = reason.trim_matches('"');
                    count(&mut refusals, reason);
                    let host = host.as_deref().unwrap_or("");
                    ("refuse", keys.clone(), host, reason.to_string())
                }
                LogLine::UpstreamError {
                    keys,
                    host,
                    about,
                    reason,
                    ..
                } => {
                    let decision = decisions.get(about);
                    if !decision.is_some_and(|(names, target)| names == keys && *target == host) {
                        flag("an upstream_error with no matching decision");
                    }
                    let reason = serde_json::to_string(reason).unwrap_or_default();
                    let reason = reason.trim_matches('"');
                    count(&mut upstream_errors, reason);
                    about_cell = about.to_string();
                    (
                        "upstream_error",
                        keys.clone(),
                        host.as_str(),
                        reason.to_string(),
                    )
                }
                LogLine::Stop { .. } => {
                    stop_seen = true;
                    ("stop", Vec::new(), "", String::new())
                }
            };
            count(&mut events, event);
            rows.push([
                seq.to_string(),
                event.to_string(),
                keys.join(",").escape_debug().to_string(),
                shown_host(host),
                reason,
                about_cell,
            ]);
        }
        if !stop_seen {
            flags.push("no stop line: a crash, a cut end, or a run still going".to_string());
        } else if let Some(number) = torn_line {
            flags.push(format!("line {number}: a line after the stop line"));
        }
    }

    let mut out = String::new();
    if !rows.is_empty() {
        let header = ["seq", "event", "keys", "host", "reason", "about"].map(String::from);
        let table: Vec<&[String; 6]> = std::iter::once(&header).chain(&rows).collect();
        let mut widths = [0; 6];
        for row in &table {
            for (width, cell) in widths.iter_mut().zip(row.iter()) {
                *width = cell.len().max(*width);
            }
        }
        let [
            seq_width,
            event_width,
            keys_width,
            host_width,
            reason_width,
            _,
        ] = widths;
        for [seq, event, keys, host, reason, about] in table {
            let row = format!(
                "{seq:>seq_width$} {event:<event_width$} {keys:<keys_width$} \
                 {host:<host_width$} {reason:<reason_width$} {about}"
            );
            out.push_str(&format!("{}\n", row.trim_end()));
        }
        out.push_str(&format!("\n{} lines: {}\n", rows.len(), joined(&events)));
        if !refusals.is_empty() {
            out.push_str(&format!("refuse: {}\n", joined(&refusals)));
        }
        if !upstream_errors.is_empty() {
            out.push_str(&format!("upstream_error: {}\n", joined(&upstream_errors)));
        }
        out.push('\n');
    }
    if let Some(number) = torn_line {
        out.push_str(&format!("line {number}: a torn last line, not judged\n"));
    }
    if flags.is_empty() {
        out.push_str("no flags\n");
    } else {
        out.push_str("flagged:\n");
        for flag in &flags {
            out.push_str(&format!("{flag}\n"));
        }
    }
    if let Some(reason) = &not_judged {
        out.push_str(&format!("not judged: {reason}\n"));
    }
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
    {
        eprintln!("write error: {error}");
        return 2;
    }

    if !flags.is_empty() {
        1
    } else if not_judged.is_some() {
        2
    } else {
        0
    }
}
