# 1. Proxy startup

**Inputs:** the command line: the paths of the settings file and the secrets file.

**Output:** an exit code. The proxy runs until a shutdown signal, then exits with success. It exits at once with a failure, saying why, if the settings or secrets can't be read or parsed, if they fail the startup checks, if this run's log file can't be created, or as soon as the log fails.

**Calls:**
- [2. Log writer](2-log-writer.md), at step 4: starts it, and logs the start line; at step 6: logs the stop line
- [4. One program connection](4-program-connection.md), at step 5: once per connection, each in its own task

**Called by:** none: the operator starts it.

## Pseudo code

The [sequence diagram](1-proxy-startup.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
function main(command_line):
    // 1. Load the settings and the secrets
    settings_bytes = read_file(command_line.settings_path)
    settings       = parse_settings(settings_bytes)                 // TOML: a repeated name, an unknown field or a name for an address is an Error
    secrets        = parse_secrets(read_file(command_line.secrets_path))
    if any of them is Error:
        print(the error)                   // the file, the line and what was wrong
        exit(failure)

    // 2. Check them
    for host in settings.hosts and key.host in settings.keys:
        host = normalize(host)            // the Parser's host[:port] rule: lowercase host:port, port written out; Error if it refuses
    for key in settings.keys:
        key.header = lowercase(key.header)
    problems = startup_problems(settings, secrets)
    if problems is not empty:
        print(problems)
        exit(failure)

    // 3. Prepare the start line
    start_line = StartLine(run             = random_id(),
                           settings_sha256 = sha256(settings_bytes))

    // 4. Create this run's log file
    log_file = create_new_file(settings.log_folder + "/" + start_line.run + ".jsonl")   // fails if it exists
    if log_file is Error:
        print(log_file.error)
        exit(failure)
    (writer, writer_queue) = bounded_queue()
    log_failed = Flag()                   // set by the Writer if a line can't be written
    writer_thread = start_thread(run_writer(log_file, writer_queue, settings.client, log_failed))   // calls 2
    if writer.submit(start_line) is Error:                                             // calls 2
        print("the log can't be written")
        exit(failure)

    // 5. Listen
    listener = listen(settings.listen_address)
    shutdown = Flag()                     // set at step 6; every connection watches it
    cut_off  = Flag()                     // set at step 6: after the time limit, or at once if the log fails
    connection_tasks = []
    loop:
        event = wait_for(a connection on listener, a shutdown signal, log_failed)
        if event is ShutdownSignal or event is LogFailed:
            break
        connection_tasks.push(start_task(handle_connection(event.program, settings, secrets, writer, shutdown, cut_off)))   // calls 4

    // 6. Shut down
    close(listener)
    if event is ShutdownSignal:
        shutdown.set()
        event = wait_for(every connection task done, 5 s passed, log_failed)
    cut_off.set()                         // the rest stop at their next wait (see 4)
    wait_for_all(connection_tasks)
    if event is LogFailed:
        exit(failure)                     // the Writer printed the error; no stop line
    if writer.submit(StopLine()) is Error:                                             // calls 2
        print("the log can't be written")
        exit(failure)
    close(writer)
    wait_for(writer_thread)
    exit(success)


// Every rule the settings and secrets break. All of them are reported.
// A secret's value is never printed, only its name.
function startup_problems(settings, secrets):
    problems = []

    for host in settings.hosts:
        if host is Error:
            problems.push("host " + host + ": not a valid host[:port]")
    for host in settings.hosts that an earlier host already gives once normalized:
        problems.push("host " + host + ": listed more than once")

    for key in settings.keys:
        if key.host not in settings.hosts:
            problems.push("key " + key.name + ": its host isn't in the host list")
        if key.placeholder doesn't have the placeholder shape:
            problems.push("key " + key.name + ": malformed placeholder")
        if key.header in [host, connection, keep-alive, te, proxy-*, content-length, transfer-encoding]:
            problems.push("key " + key.name + ": bound to a header the proxy drops, writes, or reads for framing")

    for key in settings.keys whose placeholder an earlier key already uses:   // keys in name order
        problems.push("key " + key.name + ": its placeholder is also key " + earlier.name + "'s")

    key_names    = [key.name for key in settings.keys]
    secret_names = [name for (name, value) in secrets]
    for name in key_names not in secret_names:
        problems.push("key " + name + ": no secret")
    for name in secret_names not in key_names:
        problems.push("secret " + name + ": no such key")

    for (name, value) in secrets:
        if value isn't 1 to 4096 visible ASCII characters (no spaces, \r, \n or \0):
            problems.push("secret " + name + ": invalid value")

    return problems
```

## Exits

One row per way the proxy stops, and one test per row.

| Exit | Printed | Log | Exit code |
|---|---|---|---|
| The settings or secrets file can't be read, or isn't valid TOML for its shape | the file, the line and what was wrong | nothing: the log file isn't created yet | failure |
| The settings or secrets break a startup rule | every problem, secrets by name only | nothing: the log file isn't created yet | failure |
| This run's log file can't be created (for example, it already exists) | the error | nothing | failure |
| The start line can't be written | "the log can't be written" | no complete start line | failure |
| Shutdown signal (`SIGTERM` or Ctrl-C) | nothing | the lines already queued, then the stop line | success |
| The stop line can't be written | "the log can't be written" | no stop line | failure |
| The log fails while the proxy runs | the error, printed by the Writer | no stop line | failure |
