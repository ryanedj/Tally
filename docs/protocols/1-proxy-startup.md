# 1. Proxy startup

**Inputs:** the command line: the paths of the settings file and the secrets file.

**Output:** an exit code. The proxy runs until a shutdown signal, then exits with success. It exits at once with a failure, saying why, if the settings or secrets can't be read or parsed, if they fail the startup checks, if the listen address can't be bound, if this run's log file can't be created, or as soon as the log fails. If a connection panics, it shuts down as on a signal, then exits with a failure and no stop line. If the end of the run takes more than 4 s after the cut-off (a stuck disk), it exits with a failure and no stop line, without waiting for the Writer.

**Calls:**
- [2. Log writer](2-log-writer.md), at step 5: starts it, and logs the start line; at step 7: logs the stop line
- [4. One program connection](4-program-connection.md), at step 6: once per connection, each in its own task

**Called by:** none: the operator starts it.

## Pseudo code

The [sequence diagram](1-proxy-startup.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
function main(command_line):
    // 1. Load the settings and the secrets
    if command_line doesn't give exactly two paths:
        print(the usage)
        exit(failure)
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

    // 3. Bind the listen address, listen for the shutdown signals
    listener = listen(settings.listen_address)    // before the log, so a run that can't serve leaves none
    if listener is Error:
        print(listener.error)
        exit(failure)
    signals = listen_for(SIGTERM, Ctrl-C)  // now, so that a signal sent once "listening on" is printed is caught
    if signals is Error:
        print(signals.error)
        exit(failure)

    // 4. Prepare the start line
    run = random_id()                     // 16 random bytes from the operating system
    if run is Error:
        print(run.error)
        exit(failure)
    start_line = StartLine(run, settings_sha256 = sha256(settings_bytes))

    // 5. Create this run's log file
    create_folder_if_missing(settings.log_folder)                                      // one level only
    log_file = create_new_file(settings.log_folder + "/" + start_line.run + ".jsonl")   // fails if it exists
    if log_file is Error:
        print(log_file.error)
        exit(failure)
    (writer, writer_queue) = bounded_queue()
    log_failed = Flag()                   // set by the Writer if a line can't be written, or if it crashes
    writer_thread = start_thread(run_writer(log_file, writer_queue, settings.client, log_failed))   // calls 2
    if writer_thread is Error:
        print(writer_thread.error)
        exit(failure)                     // the log file stays empty
    if writer.submit(start_line) is Error:                                             // calls 2
        close(writer)
        wait_for(writer_thread)           // the Writer prints its error first
        print("the log can't be written")
        exit(failure)
    print("listening on " + listener.address)

    // 6. Serve
    shutdown = Flag()                     // set at step 7; every connection watches it
    cut_off  = Flag()                     // set at step 7: after the time limit, or at once if the log fails
    panicked = Flag()                     // set when a connection task ends with a panic
    connection_tasks = []
    loop:
        event = wait_for(a connection on listener, a shutdown signal, log_failed, panicked)
        if event is ShutdownSignal or event is LogFailed or event is Panicked:
            break
        connection_tasks.push(start_task(handle_connection(event.program, settings, secrets, writer, log_failed, shutdown, cut_off)))   // calls 4

    // 7. Shut down
    close(listener)
    if event is not LogFailed:            // a panic shuts down as a signal does
        shutdown.set()
        wait_for(every connection task done, 5 s passed, log_failed)
    cut_off.set()                         // the rest stop at their next wait (see 4)
    deadline = now + 4 s                  // a wait for the Writer's answer is never cut off: a stuck disk would hold it
    if wait_for_all(connection_tasks, until deadline) is TimedOut:
        print("the log didn't answer in time")
        exit(failure)                     // no stop line; the Writer isn't waited for
    if log_failed is set or panicked is set:        // log_failed: by the Writer, or by the connection told first (see 4)
        if panicked is set and log_failed is not set:
            print("a connection panicked")
        close(writer)
        if wait_for(writer_thread, until deadline) is TimedOut:   // a failed Writer printed its error; no stop line
            print("the log didn't answer in time")
        exit(failure)
    stopped = writer.submit(StopLine(), until deadline)                                // calls 2
    close(writer)
    if stopped is TimedOut or wait_for(writer_thread, until deadline) is TimedOut:
        print("the log didn't answer in time")
        exit(failure)
    if stopped is Error:                  // the Writer printed its error before ending
        print("the log can't be written")
        exit(failure)
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

One row per way the proxy stops, and one test per row, except the three only a refusing operating system can cause.

| Exit | Printed | Log | Exit code |
|---|---|---|---|
| The command line doesn't give exactly two paths | the usage | nothing: the log file isn't created yet | failure |
| The settings or secrets file can't be read, or isn't valid TOML for its shape | the file, the line and what was wrong | nothing: the log file isn't created yet | failure |
| The settings or secrets break a startup rule | every problem, secrets by name only | nothing: the log file isn't created yet | failure |
| The listen address can't be bound (for example, it's in use) | the address and the error | nothing: the log file isn't created yet | failure |
| The operating system can't listen for the shutdown signals (no test) | the error | nothing: the log file isn't created yet | failure |
| The operating system gives no random bytes for the run id (no test) | the error | nothing: the log file isn't created yet | failure |
| This run's log file can't be created (for example, it already exists) | the error | nothing; an empty log file only if its folder can't be synced once the file exists | failure |
| The operating system can't start the Writer's thread (no test) | the error | an empty log file | failure |
| The start line can't be written | "the log can't be written" | no complete start line, or the start line if only its sync failed | failure |
| Shutdown signal (`SIGTERM` or Ctrl-C) | nothing | the lines already queued, then the stop line | success |
| The stop line can't be written | "the log can't be written" | no stop line, or the stop line if only its sync failed | failure |
| The log fails while the proxy runs | the error, printed by the Writer | no stop line | failure |
| A connection panics (tested on a stand-in: only a bug can cause it) | the panic, then "a connection panicked" | the lines of the connections finished as on a signal, no stop line | failure |
| The log doesn't answer within 4 s of the cut-off (tested on a stand-in: only a stuck disk can cause it) | "the log didn't answer in time" | no stop line; a line being written may be torn | failure |
