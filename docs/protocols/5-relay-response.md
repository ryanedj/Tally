# 5. Relay response

**Inputs:** given by [4. One program connection](4-program-connection.md), step 6:
- `server`: the connection the request was just sent on
- `program`: the program's connection
- `request`: the request head, for `wants_close`
- `decision`: the `release` or `pass` decision, for the keys, host and `seq` (the `about` of an `upstream_error` line)
- `writer`, `shutdown`, `cut_off`: as given to [4. One program connection](4-program-connection.md)

**Output:** `KeepOpen  |  Close`: whether the program's connection stays open after this request. [4. One program connection](4-program-connection.md) acts on it, and closes the Server connection either way.
- `KeepOpen`: the program got a complete reply of known length, without `Connection: close`.
- `Close`: the reply said `Connection: close`, or the response was cut short. [4. One program connection](4-program-connection.md) then closes the connection gently.

**Calls:** [2. Log writer](2-log-writer.md), for every line, through `upstream_failed`.

**Called by:** [4. One program connection](4-program-connection.md), at step 6.

**Uses:** the [Parser](../DESIGN.md#parser), to read and check the response head and find where its body ends (steps 2 and 4), and `upstream_failed` from [4. One program connection](4-program-connection.md).

## Pseudo code

The [sequence diagram](5-relay-response.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
function relay_response(server, program, request, decision, writer, shutdown, cut_off):
    loop:                                 // once per head: any interim (1xx) heads, then the final one
        // 1. Wait for the response head
        head_read = read_response_head(server, silence_limit = 30 s)   // each line parsed as it arrives: TimedOut, Closed, Error or Complete(response)
        if head_read is TimedOut:
            return upstream_failed(program, writer, decision, response_timeout, request, shutdown)

        // 2. Check the head
        is_bad = head_read is not Complete(response)   // the Server closed before the end of the head, or a line or the whole head breaks a rule (see DESIGN, Responses)

        // 3. Bad response
        if is_bad:
            return upstream_failed(program, writer, decision, bad_response, request, shutdown)

        // 4. Find where the response ends, rewrite the head
        body_end = where_body_ends(response)
        is_final = response.status is not 1xx
        closing  = is_final and (body_end is UntilServerCloses or request.wants_close or shutdown is set)
        send(program, rewrite_response_head(response, closing))
        if is_final:
            break

    // 5. Relay the body
    relayed = relay_body(server, program, body_end, silence_limit = 30 s)   // Done or Incomplete
    if relayed is Incomplete:
        return upstream_failed(program, writer, decision, response_incomplete, request, shutdown)

    // 6. Return to 4
    return Close if closing else KeepOpen


function where_body_ends(response):
    if response.status is 1xx or response.status in [204, 304]:
        return NoBody
    if response has Content-Length:
        return Length(response.content_length)
    return UntilServerCloses


// content-length and transfer-encoding stay: the body is relayed unchanged.
function rewrite_response_head(response, closing):
    head = response without connection, the headers it names, keep-alive and every proxy-* header
    if closing:
        head.add("Connection: close")
    return head


function relay_body(server, program, body_end, silence_limit):
    match body_end:
        NoBody:
            return Done
        Length(size):
            copy size bytes from server to program
            if the server closes, or sends nothing for silence_limit, before size bytes:
                return Incomplete
            return Done
        UntilServerCloses:
            copy from server to program until the server closes
            if the server sends nothing for silence_limit, or breaks the connection:   // RFC 9112 §8
                return Incomplete
            return Done
```
