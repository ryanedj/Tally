# 2. Log writer

**Inputs:** given by [1. Proxy startup](1-proxy-startup.md):
- `log_file`: this run's log file, newly created
- `queue`: the receiving side of the queue the connections send their lines through. Its first line is the start line and its last the stop line (the Writer refuses any line after it).
- `client`: from the settings
- `log_failed`: set by the Writer when a line can't be written, or when it stops any other way than its queue closing (a crash); [1. Proxy startup](1-proxy-startup.md) then stops the proxy at once

**Output:** an answer to each line: `OnDisk(seq)  |  Error`. The `seq` lets a connection point an `upstream_error` line back at its decision line (`about`).

**Calls:** none.

**Called by:**
- [1. Proxy startup](1-proxy-startup.md), at step 4, which starts the Writer and sends the start line, and at step 6, which sends the stop line
- [4. One program connection](4-program-connection.md) and [5. Relay response](5-relay-response.md), for every line, through `log_or_stop`

## Pseudo code

The [sequence diagram](2-log-writer.puml) is the reference. Each numbered step below details the step with the same number in it.

```text
function run_writer(log_file, queue, client, log_failed):
    next_seq = 0

    loop:
        // 1. Take the next line, in arrival order
        next_item = queue.receive()
        if next_item is QueueClosed:
            return
        (line, answer_to) = next_item

        // 2. Complete it
        line.seq    = next_seq
        line.client = client
        line.time   = now()

        // 3. Append it and force it to disk
        written = append_line(log_file, line) and force_to_disk(log_file)
        if written is Ok:
            answer_to.send(OnDisk(next_seq))
            next_seq = next_seq + 1
            if line is the stop line:     // the run's last, nothing after it is written
                queue.close()
                for (waiting_line, waiting_answer_to) in queue.take_all():
                    waiting_answer_to.send(Error)
                return
            continue

        // 4. Print the error, stop the proxy
        answer_to.send(Error)
        for (waiting_line, waiting_answer_to) in queue.take_all():
            waiting_answer_to.send(Error)
        print(written.error)
        log_failed.set()                  // stops the proxy (see 1)
        return
```
