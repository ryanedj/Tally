# 4. One program connection

**Inputs:** given to every connection by [1. Proxy startup](1-proxy-startup.md):
- `settings`: the checked settings file: `client`, the hosts with their Server addresses, and the keys (name, placeholder, allowed host, bound header in lowercase). Read-only, shared by all connections.
- `secrets`: the real key values, by key name. Only `rewrite` reads them.
- `writer`: the way to hand a line to the [Writer](2-log-writer.md). The Writer adds `seq`, `client` and `time` itself.
- `shutdown`: set by [1. Proxy startup](1-proxy-startup.md) when a shutdown signal arrives. A connection then finishes its current request and closes.
- `cut_off`: set by [1. Proxy startup](1-proxy-startup.md) when the shutdown time limit is reached. The connection then stops at its next wait (see the rule at the top of the [pseudo code](#pseudo-code)).

**Output:** none. The program's connection is closed when `handle_connection` returns.

**Calls:**
- [3. Decide request](3-decide-request.md), at step 2, to get `release`, `pass` or `refuse`
- [2. Log writer](2-log-writer.md), for every line, through `log_or_stop`
- [5. Relay response](5-relay-response.md), at step 6, to pass the Server's answer back

**Called by:** [1. Proxy startup](1-proxy-startup.md), at step 5, once per program connection, each in its own task.

**Uses:** the [Parser](../DESIGN.md#parser), to read the request head.

**Guarantees:**
- Nothing reaches the Server before the Writer has answered that the line is on disk.
- Every request the proxy starts reading gets exactly one decision line, unless the log can't be written or a shutdown cuts the request off before its line. Bytes left over once the proxy has decided to close are never handled, so they get none.
- A `release` or `pass` line is always followed by a complete send to the Server, or by an `upstream_error` line, unless the log can't be written.
- A refused request's body is never parsed or forwarded.
- Nothing carries over from one request to the next except the unread bytes in `buffer`.
- The proxy never retries. A retry by the program is a new request with its own line.

## Pseudo code

The [sequence diagram](4-program-connection.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
// Release, Pass and Refuse(Refusal) are the output of decide (see 3).
// A refusal's keys and host are empty when not known yet.
// request.content_length is 0 when there's no Content-Length header.
// request.wants_close is set when the program sent Connection: close.
// decision.seq is set once the decision's line is on disk (step 4).
// An outcome is KeepOpen or Close (see 5).
//
// Every write to program gives up after 30 s without progress:
// close(program), stop this connection, no new line.
//
// Once cut_off is set, every wait here and in 5 stops at once, except the wait for the Writer's answer:
// close(program), stop this connection, no new line.
// A connect or send cut off after its decision line logs upstream_error, shutdown first (step 5).

function handle_connection(program, settings, secrets, writer, shutdown, cut_off):
    buffer = void                         // bytes read from the program but not handled yet; each read takes from it

    loop:
        // 1. Read the head and check it follows the strict HTTP rules
        head_read = read_head(program, buffer)   // 30 s for the first byte, then 10 s and 8 KiB for the whole head
        match head_read:
            Closed               -> return                  // closed before any byte: no line
            Idle                 -> close(program); return  // 30 s of silence: no line, no reply
            ShuttingDown         -> close(program); return  // shutdown before any byte: no line, no reply
            ClosedPartWay        -> refuse(program, writer, Refusal(incomplete_request), can_reply = false); return
            TimedOut             -> refuse(program, writer, Refusal(incomplete_request)); return
            TooLarge             -> refuse(program, writer, Refusal(head_too_large)); return
            Complete(head_bytes) -> request = parse_request_head(head_bytes)

        if request is Error(reason):      // malformed_request, unsupported_request, head_too_large (over 100 headers) or body_too_large (Content-Length over 1 MiB)
            refuse(program, writer, Refusal(reason))
            return

        // 2. Decide the request: the first case that applies
        decision = decide(request, settings)                  // calls 3
        if decision is Refuse(refusal):
            refuse(program, writer, refusal)
            return

        // 4. Read the rest of the body, then log the decision
        body_read = read_body(program, buffer, request.content_length)   // 30 s in total
        match body_read:
            ClosedPartWay  -> refuse(program, writer, Refusal(incomplete_request, decision.keys, decision.host), can_reply = false); return
            TimedOut       -> refuse(program, writer, Refusal(incomplete_request, decision.keys, decision.host)); return
            Complete(body) -> (go on)

        decision.seq = log_or_stop(program, writer, decision)

        // 5. Rewrite the request and swap each placeholder for its real key
        upstream_head = rewrite(request, decision, settings, secrets)   // (see DESIGN, Handling one request, steps 6 and 7)
        server = connect(settings.address_of(decision.host))  // 5 s, never DNS
        if server is Error(reason):                           // connect_failed or connect_timeout; shutdown if cut off
            outcome = upstream_failed(program, writer, decision, reason, request, shutdown)
        else:
            sent = send(server, upstream_head, body)
            if sent is Error(reason):                         // send_failed; shutdown if cut off
                outcome = upstream_failed(program, writer, decision, reason, request, shutdown)
            else:
                // 6. Relay the response
                outcome = relay_response(server, program, request, decision, writer, shutdown, cut_off)   // calls 5
            close(server)                                     // one Server connection per request

        // 7. Keep the connection for the next request
        if outcome is Close:
            close(program)                                    // leftover bytes are never handled
            return


function log_or_stop(program, writer, line, can_reply = true):
    answer = writer.submit(line)          // calls 2: waits until the line is on disk; Error if the queue is closed
    if answer is Error:
        if can_reply:                     // false once a final response head was relayed (see 5), or the program has closed
            reply(program, 503, closing = true)
        close(program)
        stop this connection              // the proxy stops (see 1)
    return answer.seq


// 3. Refuse
function refuse(program, writer, refusal, can_reply = true):
    log_or_stop(program, writer, refusal, can_reply)
    if not can_reply:                     // the program already closed its side
        close(program)
        return
    reply(program, status_of(refusal.reason), closing = true, body = refusal.reason)
    close_gently(program)


// The upstream_error line after a release or pass, then the reply its reason has, if any.
function upstream_failed(program, writer, decision, reason, request, shutdown):
    log_or_stop(program, writer, UpstreamError(reason, decision.keys, decision.host, about = decision.seq),
                can_reply = reason is not response_incomplete)
    closing = reason in [bad_response, response_incomplete, shutdown] or request.wants_close or shutdown is set
    status = status_of(reason)            // 502 or 504; none for shutdown and response_incomplete
    if status is not none:
        reply(program, status, closing)
    return Close if closing else KeepOpen


// Every reply the proxy writes itself, with Connection: close when the connection closes after it.
function reply(program, status, closing, body = empty):
    head = status line and Content-Length of body
    if closing:
        head.add("Connection: close")
    send(program, head, body)


function close_gently(program):
    stop writing to program
    read and discard what the program sends, for at most 1 s or 1 MiB
    close(program)
```

## Exits

One row per way out of a loop turn, and one test per row.

| Exit | Log line | Reply | Program connection |
|---|---|---|---|
| Program closes before a request's first byte | none | none | closed |
| 30 s of silence before a request's first byte | none | none | closed |
| Shutdown before a request's first byte | none | none | closed |
| Program closes part-way through the head | `refuse`, `incomplete_request` | none | closed |
| Head not complete within 10 s | `refuse`, `incomplete_request` | 408 | closed gently |
| Head over 8 KiB or 100 headers | `refuse`, `head_too_large` | 431 | closed gently |
| `Content-Length` over 1 MiB | `refuse`, `body_too_large` | 413 | closed gently |
| Parse error | `refuse`, `malformed_request` or `unsupported_request` | 400 or 501 | closed gently |
| Refused by `decide` | `refuse`, its reason | 400 or 403 | closed gently |
| Program closes part-way through the body | `refuse`, `incomplete_request`, with keys and host | none | closed |
| Body not complete within 30 s | `refuse`, `incomplete_request`, with keys and host | 408 | closed gently |
| The log can't be written | none (printed by the Writer) | 503, unless a final response head was relayed | closed; the proxy stops |
| The connection to the Server fails | `release`/`pass`, then `upstream_error`, `connect_failed` | 502 | open, unless `Connection: close` or a shutdown |
| No connection within 5 s | `release`/`pass`, then `upstream_error`, `connect_timeout` | 504 | open, unless `Connection: close` or a shutdown |
| Sending to the Server fails | `release`/`pass`, then `upstream_error`, `send_failed` | 502 | open, unless `Connection: close` or a shutdown |
| Bad response, or the Server closes before the end of its head (see [5. Relay response](5-relay-response.md)) | `release`/`pass`, then `upstream_error`, `bad_response` | 502 | closed |
| 30 s with nothing from the Server, before the final response head (see [5. Relay response](5-relay-response.md)) | `release`/`pass`, then `upstream_error`, `response_timeout` | 504 | open, unless `Connection: close` or a shutdown |
| Response cut short: the Server closes or goes silent after the head (see [5. Relay response](5-relay-response.md)) | `release`/`pass`, then `upstream_error`, `response_incomplete` | none: the response stops | closed |
| Response with a known length (`Content-Length`, `204`, `304`) | `release`/`pass` | relayed | open, unless `Connection: close` or a shutdown |
| Chunked response, or no length | `release`/`pass` | relayed until the Server closes | closed |
| Program stops reading for 30 s | `release`/`pass` | cut off | closed |
| Shutdown time limit while connecting or sending to the Server | `release`/`pass`, then `upstream_error`, `shutdown` | none | closed |
| Shutdown time limit at any other wait | nothing new | none | closed |
