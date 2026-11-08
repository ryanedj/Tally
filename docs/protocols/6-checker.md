# 6. Checker

**Inputs:** the command line: the paths of a run's log file and of a settings file.

**Output:** a report (one row per line of a judged run, counts and flags, whether the run was judged, a torn last line), and an exit code: 1 if anything is flagged, otherwise 2 if the run couldn't be judged, otherwise 0.

**Calls:** none.

**Called by:** none: the auditor runs it, by hand or from a test.

## Pseudo code

The [sequence diagram](6-checker.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
function check(command_line):
    report = Report()

    // 1. Fingerprint the settings file
    if command_line doesn't give exactly two paths:
        print(the usage)
        exit(2)
    settings_bytes  = read_file(command_line.settings_path)
    if settings_bytes is Error:
        print("can't read the settings file: not judged")
        exit(2)
    settings_sha256 = sha256(settings_bytes)

    // 2. Read the run's log
    (lines, torn_last_line) = read_lines(command_line.log_path)
    if lines is Error:
        print("can't read the log file: not judged")
        exit(2)
    if torn_last_line is not none:
        report.note("last line torn: not judged")

    run_lines = []
    for (line_number, text) in lines:
        line = parse_log_line(text)       // a known event, with exactly its fields (see DESIGN, The log)
        if line is Error:
            report.flag(line_number, "malformed line")
        else:
            run_lines.push(line)

    // 3. Not judged: no start line, different settings, or settings that aren't valid
    if run_lines is empty or run_lines[0].event is not start:
        report.flag("the first line isn't a start line: not judged")
    else if run_lines[0].settings_sha256 != settings_sha256:
        report.not_judged("different settings")
    else:
        settings = check_settings(parse_settings(settings_bytes))   // the rules on the settings file alone
        if settings is Error:
            report.not_judged("settings not valid")
        else:
            check_lines(run_lines, torn_last_line, settings, report)

    // 5. Report
    if print(report) is Error:             // a reader that closed early, say
        print("write error")               // on stderr
        exit(2)
    if report has flags:
        exit(1)
    if report has not_judged:
        exit(2)
    exit(0)


// 4. Check every line
function check_lines(run_lines, torn_last_line, settings, report):
    decisions_by_seq = {}
    stop_seen        = false
    for (position, line) in enumerate(run_lines):
        if stop_seen:
            report.flag(line, "line after the stop line")
        expected_seq = 0 if position == 0 else run_lines[position - 1].seq + 1
        if line.seq != expected_seq:
            report.flag(line, "seq gap, repeat or step back")
        if line.client != settings.client:
            report.flag(line, "wrong client")
        match line.event:
            start:
                if position != 0:
                    report.flag(line, "second start line")
            release:
                for key_name in line.keys:
                    key = settings.key_named(key_name)
                    if key is none or key.host != line.host:
                        report.flag(line, "release not allowed by the settings")
                decisions_by_seq[line.seq] = line
            pass:
                if line.host not in settings.hosts:
                    report.flag(line, "pass not allowed by the settings")
                decisions_by_seq[line.seq] = line
            upstream_error:
                decision = decisions_by_seq.get(line.about)
                if decision is none or decision.keys != line.keys or decision.host != line.host:
                    report.flag(line, "upstream_error with no matching decision")
            stop:
                stop_seen = true
        report.row(line)
        report.count(line.event, line.reason)

    if not stop_seen:
        report.flag("no stop line: a crash, a cut end, or a run still going")
    else if torn_last_line is not none:
        report.flag(torn_last_line, "line after the stop line")
```

## Exits

One row per thing the checker reports, and one test per row.

| Exit | Report | Exit code |
|---|---|---|
| The run ends with its stop line, and every line follows the settings | rows and counts | 0 |
| A line that isn't valid JSON, or doesn't have its event's exact fields (for example a `release` with no keys, or a `pass` with some) | flagged: malformed line | 1 |
| The first line isn't a start line | flagged; nothing else is judged | 1 |
| A second start line | flagged | 1 |
| A `seq` gap, repeat or step back | flagged | 1 |
| A line whose `client` isn't the settings' client | flagged | 1 |
| A `release` whose key isn't allowed for its host | flagged | 1 |
| A `pass` to a host that isn't listed | flagged | 1 |
| An `upstream_error` with no matching earlier `release` or `pass` | flagged | 1 |
| No stop line (a crash, a cut end, or a run still going) | flagged | 1 |
| A line after the stop line | flagged | 1 |
| A last line without `\n`, anywhere but after the stop line | noted as torn, not judged | no effect |
| A last line without `\n` after the stop line | noted as torn, and flagged as a line after the stop line | 1 |
| The settings file doesn't match the run | not judged | 2, unless something is flagged |
| The settings file matches the run but isn't valid (it doesn't parse, or breaks a rule on the settings file) | not judged | 2, unless something is flagged |
| The log or settings file can't be read | not judged | 2 |
| The command line doesn't give exactly two paths | the usage | 2 |
| The report can't be written, such as a reader that closed early | `write error` on stderr | 2 |
