# Tally
A secure proxy that controls, logs, and audits API-key access from sandboxed programs.

Status: v1 design done, last changed 2026-10-18. Build in progress.

## What Tally does

A program runs in a sandbox and needs to call an outside API with an API key. The program never gets the real key, only a placeholder. All its traffic goes through the Tally proxy, which decides every request:

1. it checks where the request is going, and that any key it carries is allowed to go there;
2. it writes its decision in a log, and makes sure the line is on disk;
3. only then does it replace the placeholder with the real key and send the request.

Refused requests are logged too. Later, a checker reads the log to see every use of every key and every refused attempt, and to catch any release that broke the rules.

## v1 goals

What I want to learn in v1:
- Networking in Rust (tokio) and HTTP, with a parser I write myself
- Docker and Docker Compose
- Creating and using Claude skills

What v1 must do:
- Everything runs locally: a fake API, a fake program, the proxy, a log file, and a checker.
- A key is released only toward its allowed host, and only after its log line is on disk.
- Every request the proxy starts reading gets exactly one decision in the log, unless the log can't be written (then the proxy stops) or a shutdown cuts the request off before its line (then nothing was sent). Refusals carry a reason.
- The checker can flag any logged release that broke the rules.

## v1 non-goals

- No encryption, plain HTTP only.
- Anyone with access to the log can change or delete its lines. The checker catches naive edits (a missing line leaves a gap in `seq`; a cut end leaves no `stop` line, as a crash does) but not careful ones.
- No isolation with plain `cargo`: the fake program runs as the same user, so it could read the secrets file or the log, or contact the Server directly. The Docker step (L8) adds the isolation.
- The program can flood the log. Every refusal is a line forced to disk, so the program can fill the disk or slow the proxy down.
- Only one program per proxy (named in the settings), and only one kind of request body (`Content-Length`).

## v1 assumptions

- The proxy is trusted: it holds the real keys, so if it were compromised it could send one without logging it.
- Allowed hosts are trusted with the key: they receive it, so they could always misuse it. Tally only keeps the key in its bound header, out of URLs, which an honest server writes to its logs. It can't stop a server that echoes the key: responses are relayed unfiltered, so a server that reflects a key's header (a debug endpoint, a verbose error page) hands the key back to the program.
- Nothing else writes to a run's log file. The proxy has a single writer, so lines and `seq` stay consistent only if no other process appends to it.

## Established practices

I first meant to work everything out on my own, thinking it would teach me more. Then I realized that writing the protocols already gives me experience, and that designing a protocol ad hoc, without looking at how others solved the same problems, is a mistake. So before a protocol is final, I compare it with how the projects below handle the same cases.

| Source | What it is | What Tally takes from it |
|---|---|---|
| nginx | The most common reverse proxy | The 1 MiB body limit; reading the whole body before contacting the server; reading and discarding what the client still sends before closing (`lingering_close`) |
| Go's `net/http` and `httputil.ReverseProxy` | A server and a proxy with well-documented timeouts | Idle wait and head deadline; closing the write side before closing the connection; when a connection can be reused; graceful shutdown (close idle connections at once, finish active ones without keep-alive); `Connection: close` on every reply before a close; forwarding interim (`1xx`) responses; 502 when the server gives no usable response; an empty path sent as `/`; a response's `Connection` list read with its empty elements skipped; a head judged line by line, as each line arrives |
| hyper | The HTTP library under most Rust servers | Strict defaults; status codes for oversized heads; the syntax checked before the meaning; a response's checks and where its body ends decided with its head |
| Envoy | A proxy for service-to-service traffic, with a credential injector | Connect timeout; operator-chosen header for the credential; unknown fields in static config refused |
| Pingora | Cloudflare's Rust proxy framework | Its security fixes, as a list of mistakes to avoid |
| Smokescreen (Stripe) | An egress proxy that decides which hosts a program may reach | Per-decision logging; checking addresses, not names |
| Fly.io's tokenizer | A proxy that adds secrets to requests from untrusted programs | The closest match to Tally: host allowlists per secret, no reuse of server connections, and the allowlist as its answer to a service that echoes secrets |
| iron-proxy | An egress proxy that swaps proxy tokens from a sandbox for real secrets | Swapping a token only where it's configured to go: headers by default, the URL only on request, since URLs end up in access logs |
| RFC 6750 (OAuth 2.0 bearer tokens) | How a client sends a bearer token | Keep keys out of URLs, since URLs end up in logs (§2.3, §5.3) |
| `httparse` | Rust's strict HTTP/1 head parser | A cross-check for Tally's own parser in tests; the parser's shape (the bytes read so far, read again from the start, give a partial head, a complete one, or an error); one named test per table row |
| Schneier and Kelsey, *Secure Audit Logs to Support Computer Forensics* (1999) | A design for audit logs kept on a machine that may be taken over | One file per session, opened and closed by special entries; and the reason a crash and a cut end look the same without cryptography |
| systemd-journald | Linux's system log service | After an unclean stop, a new file instead of appending to the old one |
| PostgreSQL | A database that forces its write-ahead log to disk | Stop at once when a sync fails, never retry (`data_sync_retry` off) |

## Decisions

### The log

- Each run has its own file. At startup the proxy creates it in the log folder, named after the run id (`<run>.jsonl`), and never appends to an existing file. *Why:* after a crash the last line can be torn, and a new run appending to that file would glue its start line onto it. One file per run, opened by a `start` line and closed by a `stop` line, is the design of Schneier and Kelsey's secure audit logs; journald also starts a new file after an unclean stop.
- One JSON object per line, with these fields:
  - `seq`: the line's number in the run, counted by the Writer; `0` is the `start` line
  - `event`: `start`, `release`, `pass`, `refuse`, `upstream_error`, or `stop`
  - `client`: the program the proxy serves
  - `keys`: the names of the keys whose placeholder is in their bound header (empty when there are none)
  - `host`: where the request is going, lowercase, as `host:port` (`null` when the request couldn't be read that far)
  - `reason`: on `refuse` and `upstream_error` only
  - `about`: on `upstream_error` only: the `seq` of the `release` or `pass` line it follows. *Why:* several connections log at once, so without it the checker can't tell which request failed.
  - `time`: for information only
- A `start` line when the proxy starts, with `run` (a random id), `client`, and `settings_sha256` (a SHA-256 of the settings file's bytes, as on disk).
- A `stop` line when the proxy shuts down cleanly, after every other line of the run. *Why:* without it, lines cut from the end of a run would leave no trace. With it, a cut end looks like a crash: both lack the `stop` line, and the checker flags both.
- Ordering comes from `seq`, not `time`.
- The secret never appears in the log.
- `release` means "allowed, and the key is about to be sent". If sending the request or reading the response fails afterwards, an `upstream_error` line follows, as it does after a `pass`.

Fields per event, besides `seq`, `event`, `client` and `time`, which every line has. A line with any other shape, or an unknown event or reason, is malformed.

| Event | Fields |
|---|---|
| `start` | `run`, `settings_sha256` |
| `release` | `keys` (at least one), `host` |
| `pass` | `keys` (empty), `host` |
| `refuse` | `keys`, `host` (or `null`), `reason` |
| `upstream_error` | `keys`, `host`, `about`, `reason` |
| `stop` | none |

Example:
```
{"seq":0,"event":"start","run":"8c1f0a3e9b7d4c62","client":"weather-app","settings_sha256":"9f2c…","time":"2026-09-26T10:15:00Z"}
{"seq":1,"event":"release","client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:02Z"}
{"seq":2,"event":"refuse","client":"weather-app","keys":["weather_key"],"host":"evil.local:80","reason":"host_not_allowed","time":"2026-09-26T10:15:09Z"}
{"seq":3,"event":"refuse","client":"weather-app","keys":[],"host":null,"reason":"malformed_request","time":"2026-09-26T10:15:11Z"}
{"seq":4,"event":"upstream_error","client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","about":1,"reason":"response_timeout","time":"2026-09-26T10:15:32Z"}
{"seq":5,"event":"stop","client":"weather-app","time":"2026-09-26T18:00:00Z"}
```

### The log writer

- One writer owns the run's log file, created new at startup and only appended to. Connections send it their line and wait for its answer.
- The Writer answers only once the line is on disk; the connection acts only after that answer.
- Any write or sync error stops the proxy for good, without a retry. Waiting requests get `503`, or just a closed connection once their final response head was relayed. The error is printed, and the proxy exits with a failure: requests in progress are cut off at once, since nothing more can be logged, and there is no `stop` line. *Why:* after a failed sync, a retry can report success for lines that never reached the disk.
- Lines wait in a queue of limited size, and are never dropped. When the queue is full, a connection waits for room before its line is even queued. A full queue doesn't stop the proxy.

### Shutdown

On `SIGTERM` or Ctrl-C, the proxy:
1. stops accepting new program connections;
2. closes the connections waiting for their next request at once; lets the requests in progress finish, with `Connection: close` on their response, then closes their connections;
3. after 5 s, cuts off whatever is left: a request still connecting or sending to the Server, after its `release` or `pass` line, logs `upstream_error`, `shutdown`; every other connection just closes;
4. once every connection is done and the Writer has answered every queued line, logs the `stop` line, the run's last, and exits.

*Why:* Docker sends `SIGKILL` 10 s after `SIGTERM`. A request whose line wasn't written by then had nothing sent, so the log is still correct. A request whose line was written but whose send was cut off gets its `upstream_error`, so every `release` still ends in a send or a recorded failure.

### Handling one request

1. Parse the request head with the strict parser (see [Parser](#parser)).
2. Where is it going? Only the full-URL form is accepted: `GET http://host[:port]/path?query HTTP/1.1`, which is what programs send to a proxy. The URL's host and the single `Host` header must agree, compared case-insensitively, with `:80` added when the port is missing. Otherwise: `host_mismatch`.
3. Find the keys, and check them. Each key is bound to one header in the settings (e.g. `Authorization`), and counts only when its placeholder appears in that header. The same text anywhere else (the URL, another header, the body) is forwarded unchanged: only bound headers are swapped, and a placeholder isn't secret. A key that isn't allowed for this host is `host_not_allowed`, and the whole request is refused with one reason: the first that applies, in the order of [3. Decide request](protocols/3-decide-request.md). *Why:* a half-swapped request would send one key while refusing another.
4. No key at all: the request is forwarded only to a host listed in the settings, and logged as `pass`. Any other host: `host_not_allowed`.
5. Read the whole body before logging a `release` or `pass` (at most 1 MiB, within 30 s). The decision uses the head alone; the body only has to arrive in full. A refused request's body is never parsed or forwarded. *Why:* a `release` line must always be followed by a real send, or by an `upstream_error`. If the proxy logged first, a program that sends the head and then stalls would leave a `release` for a request that was never sent.
   - If the body stops arriving (timeout, or the program closes part-way), the request is refused as `incomplete_request`. The line keeps the keys and host the decision found, so the auditor sees which key was about to leave.
6. Log first, then send. The decision line goes through the Writer and is on disk before the request is rewritten or sent. Then each key's placeholder is swapped for the key, every occurrence, in its bound header only, and the request is sent. If the log can't be written, nothing is sent. *Why:* a crash between the two steps must never leave a sent key without a record.
7. Rewrite the request for the Server.
   - The request line becomes the short form: `GET /path?query HTTP/1.1`.
   - The proxy writes its own `Host` (from the URL) and `Connection: close`.
   - It drops the connection-level headers the program sent: `Connection`, `Keep-Alive`, `TE`, and every `Proxy-*` header.
   - The program's `Connection` header may only say `close` or `keep-alive`; anything else is `unsupported_request`, checked with the head at step 1. *Why:* a header named in `Connection` would be dropped here, after the decision. With `Connection: Authorization`, the log would say a key was released when it was never sent.
   - The Server's address comes from the settings, never from DNS.
   - *Why:* the Server must see exactly the request the proxy checked, and a DNS lookup could send a checked name to an unchecked address.
8. Refusal replies: status code and reason as in the table below, then the connection is closed gently. The proxy stops writing, then reads and discards what the program already sent, for at most 1 s or 1 MiB, before closing. *Why:* closing with unread data can make the operating system send a reset that erases the reply before the program reads it (RFC 9112 §9.6). When the program has already closed its side, there is nobody to reply to: the line is logged and the connection closed.

### Decisions and reasons

| Event, reason | When | Reply |
|---|---|---|
| `release` | At least one key found in its bound header, and every key found is allowed for this host | forwarded |
| `pass` | No key found; the host is listed in the settings | forwarded |
| `refuse`, `incomplete_request` | The head or the body stopped arriving: timed out, or the program closed part-way | 408 on a timeout; none if the program closed |
| `refuse`, `head_too_large` | A head over 8 KiB, or more than 100 headers | 431 |
| `refuse`, `body_too_large` | A `Content-Length` over 1 MiB, refused before any body is read | 413 |
| `refuse`, `malformed_request` | Anything else outside the strict subset, including a short-form (`/path`) target, or the bound header of a key found appearing more than once (RFC 9110 §5.3) | 400 |
| `refuse`, `host_mismatch` | URL host and `Host` disagree, or zero or several `Host` headers | 400 |
| `refuse`, `unsupported_request` | Valid HTTP that v1 doesn't handle: `Transfer-Encoding`, `Upgrade`, `Expect`, HTTP/1.0, a scheme other than `http`, a `Connection` header other than one line with a single `close` or `keep-alive`, methods other than `GET`, `POST`, `PUT`, `PATCH`, `DELETE` | 501 |
| `refuse`, `host_not_allowed` | A key found isn't allowed for this host, or no key is found and the host isn't listed | 403 |
| `upstream_error`, `connect_failed` | The connection to the Server failed | 502 |
| `upstream_error`, `connect_timeout` | No connection to the Server within 5 s | 504 |
| `upstream_error`, `send_failed` | The connection broke while sending the request | 502 |
| `upstream_error`, `shutdown` | The shutdown time limit cut off the connect or the send; some bytes may have reached the Server | none, connection closed |
| `upstream_error`, `bad_response` | The Server's response stops before the end of its head, or breaks the strict rules (see [Responses](#responses)) | 502 |
| `upstream_error`, `response_timeout` | 30 s with nothing from the Server, before the final response head | 504 |
| `upstream_error`, `response_incomplete` | The Server closes before the announced length, or goes silent for 30 s, after the final head was relayed | none, connection closed |
| *(not logged)* | The log itself can't be written | 503 if no final head was relayed, then the proxy exits |

### Placeholders

- Shape: `TALLY_` + 32 lowercase hex characters.

### Parser

- Hand-written, strict subset of HTTP/1.1. The same parser reads request heads and response heads.
- Header names are case-insensitive (RFC 9110 §5.1). The parser lowercases them, and startup lowercases each key's bound header, so every comparison after that (`Host`, bound headers, duplicates, dropped headers) is exact. Heads are forwarded with lowercase names, both ways. *Why:* one place to get case right, instead of every comparison; Rust's `http` crate does the same.
- Each line is judged once it has fully arrived; a line that doesn't end within 8 KiB is `head_too_large`, whatever it holds. The syntax is checked first, then the meaning: when a head breaks rules of both kinds, the syntax's reason is logged (`malformed_request` or `head_too_large`); within each kind, the first faulty line wins. *Why:* the same syntax code reads requests and responses, and each side adds its own checks after it, as in hyper. Go's `net/textproto` also reads heads line by line. So a peer that stalls in the middle of a bad line is refused for its timeout (`incomplete_request` or `response_timeout`).
- Refused as `malformed_request` (`bad_response` in a response):
  - a line ending that isn't `\r\n`, or a lone `\r` or `\n` anywhere
  - a request line that isn't `METHOD target VERSION` with exactly one space between the parts, or a version other than `HTTP/1.1` (`HTTP/1.0` is refused as `unsupported_request` instead)
  - a status line that isn't `HTTP/1.1`, a code from 100 to 599, and a reason phrase that may be empty, with one space between each (RFC 9112 §4, RFC 9110 §15)
  - an empty line before the request line, or whitespace between it and the first header
  - a header name with characters outside the token set (RFC 9110 §5.6.2), or a space before the colon
  - a header continued on the next line
  - control characters other than tab in a header value
  - a `Content-Length` that isn't plain digits (no sign, no leading zeros except `0`), or appears twice
  - a short-form target (`/path`), or any other target that isn't `scheme://host[:port]` with an optional path and query
  - a URL host with user info (`user@`), a trailing dot, percent-encoding, or characters other than letters, digits, `-` and `.`
  - an empty host, a port with characters other than digits, or a port of 0 or over 65535
  - a path or query with characters RFC 3986 doesn't allow there (§3.3, §3.4), a `%` not followed by two hex digits, or a `#`
- Accepted as the RFCs allow: the scheme `http` in any case (RFC 3986 §3.1); an empty path, sent as `/` (RFC 9112 §3.2.1); an empty port, which means 80, and leading zeros in a port (RFC 9110 §4.2.1); a tab inside a header value and bytes 0x80 to 0xFF, kept as they are, while spaces and tabs around a value are dropped (RFC 9110 §5.5).
- Refused with their own reasons: `head_too_large`, `body_too_large` and `unsupported_request`; see the table above.

### Timeouts

- 30 s of silence before a request's first byte: the connection is closed, with no reply and no line (RFC 9112 §9.5 lets either side close an idle connection).
- 10 s for the whole head, counted from its first byte: `incomplete_request`. A program sending one byte at a time can't stretch it.
- 30 s for the whole body: `incomplete_request`.
- 5 s to connect to the Server: `connect_timeout`.
- 30 s without progress on a write to the program (the program stopped reading): the connection is closed. No new line: the request already has its line.
- 30 s with nothing from the Server while waiting for or relaying its response: `response_timeout` before the final head, `response_incomplete` after it.

### Bodies and connections

- Request bodies: only one `Content-Length` header. Any `Transfer-Encoding` is refused as `unsupported_request`.
- Several requests per program connection. Each request is checked from scratch; nothing carries over from the previous one. The next request starts exactly after `Content-Length` bytes of body.
- The program's connection stays open after a request only if all of these hold:
  - the request wasn't refused, so its body was read in full;
  - the program got a reply with a known length: the proxy's own reply, or a relayed response with `Content-Length`, `204` or `304`;
  - the program didn't send `Connection: close`;
  - the proxy isn't shutting down.

  Otherwise the connection closes. So it stays open after `connect_failed`, `connect_timeout`, `send_failed` and `response_timeout`, and closes after every refusal, after `bad_response` and after `response_incomplete`.
- Every reply to the program, relayed or the proxy's own, carries `Connection: close` when the proxy will close the connection after it, and no `Connection` header otherwise. *Why:* RFC 9112 §9.6: "A sender SHOULD send a Connection header field … containing the 'close' connection option when it intends to close a connection."
- Once the proxy has decided to close, it never handles the bytes left in the buffer, even a complete next request. They get no line. *Why:* RFC 9112 §9.6: after a close, the server must not process further requests on that connection; the program is expected to send them again.
- Toward the Server: one new connection per request, with `Connection: close`. *Why:* keeping connections open means the proxy must always know exactly where each request ends. Any doubt is a smuggling risk, so every unclear case ends the connection.

### Responses

- Every response head is parsed with the same strict parser.
- Interim responses (`1xx` other than `101`) are relayed to the program as they arrive, rewritten like a final head but without a `Connection` header of the proxy's own; the proxy then waits for the next head. *Why:* RFC 9110 §15.2: "A proxy MUST forward 1xx responses unless the proxy itself requested the generation of the 1xx response."
- A response's `Connection` header is read as a list (RFC 9110 §5.6.1): every line is split on commas, empty elements are skipped, and each other element must be a token (otherwise `bad_response`), lowercased. *Why:* unlike the program, a server may name any header there, since dropping it changes nothing the proxy decided. Go's `ReverseProxy` reads it the same way.
- `502` and `upstream_error`, `bad_response`, both connections closed, when a response:
  - breaks the strict rules
  - stops before the end of its head (the Server closes), or has a head over 8 KiB or with more than 100 headers, the same limits as for requests
  - is a `101`: v1 refuses `Upgrade`, and a server must not switch to a protocol the request didn't ask for (RFC 9110 §7.8)
  - is a `1xx` or a `204` with `Content-Length` or `Transfer-Encoding`, which a server must not send (RFC 9110 §8.6, RFC 9112 §6.1): both end with their head
  - has both `Content-Length` and `Transfer-Encoding`
  - has an invalid `Content-Length`
  - has a `Connection` header naming `content-length` or `transfer-encoding`. *Why:* RFC 9110 §7.6.1 forbids it, and since the body is relayed unchanged, dropping either header would change where the program thinks the response ends.
- Where the response ends (RFC 9112 §6.3):
  - `204` and `304`: at the end of the head; there is no body.
  - With `Content-Length`: after exactly that many bytes. The program's connection can stay open (see [Bodies and connections](#bodies-and-connections)).
  - Chunked, or no length: the proxy relays until the Server closes, then closes the program's connection too.
- A response cut short (the Server closes or goes silent before all `Content-Length` bytes arrive): both connections are closed without finishing the response, and `upstream_error`, `response_incomplete` is logged. The program receives fewer bytes than announced, so it knows the response is incomplete (RFC 9112 §6.3, §8). A chunked or length-less response is relayed until the Server closes, without reading its framing, so the proxy can't tell a cut-short one from a complete one; it only notices silence.
- On a relayed head, the proxy drops `connection`, the headers it names, `keep-alive` and every `proxy-*` header, then writes its own `Connection` header by the rule in [Bodies and connections](#bodies-and-connections). It keeps `content-length` and `transfer-encoding`: the body is relayed unchanged, so its framing must stay.

### Settings and secrets

- Settings file, which can be committed:
  - `client`: the program this proxy serves
  - listen address and log folder
  - hosts, each as `host:port`, with the Server's address to connect to (e.g. `127.0.0.1:9001`). A host may be written in any spelling the parser accepts for a URL host, it's normalized to lowercase `host:port`, port written out.
  - keys, each with a name, a placeholder, an allowed host, and a bound header
- Secrets file: real key values only, by key name.
- Both files are TOML. Hosts and keys are tables keyed by name, so TOML itself refuses a name written twice, and a field the proxy doesn't know is refused too:
  ```toml
  # settings.toml
  client = "weather-app"
  listen_address = "127.0.0.1:8080"
  log_folder = "logs"

  [hosts]
  "api.weather.local:80" = "127.0.0.1:9001"

  [keys.weather_key]
  placeholder = "TALLY_0123456789abcdef0123456789abcdef"
  host = "api.weather.local:80"
  header = "Authorization"
  ```
  ```toml
  # secrets.toml
  weather_key = "the-real-key"
  ```
- Command line: the paths of both files.
- A file that can't be read or isn't valid stops the proxy, which says which file, which line and what was wrong but never prints the line as the two files could be swapped on the command line. For the secrets file, only the line, since even the message can quote a value (serde's ``invalid type: integer `12345` ``). For the settings file, what was wrong is toml's message: it names fields, but quotes a value of the wrong type, so a secrets file passed as the settings file would show a secret if a key is named `hosts` or `keys` (the two settings fields that expect a table).
- Checked at startup. The proxy refuses to start if:
  - a host isn't a valid `host[:port]`, or is listed twice once normalized
  - a key's host isn't in the host list
  - a placeholder has the wrong shape or is used twice
  - a key is bound to a header the proxy drops, writes itself, or reads for framing (`Host`, `Connection`, `Keep-Alive`, `TE`, `Proxy-*`, `Content-Length`, `Transfer-Encoding`)
  - a secret isn't 1 to 4096 visible ASCII characters (no spaces, `\r`, `\n` or `\0`)
  - a key has no secret, or a secret names no key
- *Why:*
  - The command line is visible to other programs, so the secret can't go there.
  - Environment variables leak into child processes and crash reports.
  - Splitting the files keeps the secrets away from everything that can be shared.
  - `client` is set by the operator, never by the program, which could lie about its own name. Keeping it in the settings lets the checker read it.
  - A secret ending in a newline (e.g. written with `echo`) would split the header it's swapped into and smuggle a second request to the Server.

### Checker

- Reads one run's log file and the settings file.
- Meant for finished runs: a run still going has no `stop` line yet, and is flagged.
- Refuses to judge a run whose `settings_sha256` doesn't match the settings file it was given.
- Lists and counts the run's events.
- Flags:
  - any `release` or `pass` that breaks the settings (key not allowed for the host, host not listed)
  - any line whose `client` isn't the settings' client
  - gaps, repeats, or steps backwards in `seq`, each line compared with the one before it, so one missing line gives one flag
  - a first line that isn't a `start` line, or a second `start` line
  - a missing `stop` line (a crash or a cut end), or any line after it
  - an `upstream_error` whose `about` isn't an earlier `release` or `pass` line, with the same keys and host
  - a malformed complete line (not valid JSON, an unknown event or reason, or fields that don't match its event, such as a `release` with no keys); a torn last line is reported separately, as below
- A last line that doesn't end in `\n` isn't judged, only reported as torn: a crash in the middle of a write, or a run still being written. Reading needs no lock, because the proxy only appends.
- Exit code: 1 when something is flagged; otherwise 2 when the run couldn't be judged (settings that don't match the run, or a file that can't be read); otherwise 0.

### Cargo workspace

- Shared library + separate binaries: one `tally-core` library crate for shared code (parser, checks, log format, settings loading), and four binary crates (`fake-api`, `proxy`, `fake-program`, `checker`). All but the fake API depend on it: the fake API reads requests with `httparse`, so a bug in Tally's parser can't hide in the tests meant to catch it.

### Tests

- `tally-core`'s parser tables, settings checks and decisions are integration tests in `tally-core/tests/it/`, one module per topic, since they use only its public functions. Unit tests (`#[cfg(test)] mod tests` inside a source file) only for a private helper a public function can't reach.
- Integration tests: one test binary per crate, `tests/it/main.rs`, with one module per topic.
- Every parser test input also goes through `httparse` (a dev-dependency of `tally-core`; only the fake API uses it in its own code). Tally must never accept a head that `httparse` rejects.
- One test per row of the exits tables, in [1. Proxy startup](protocols/1-proxy-startup.md), [4. One program connection](protocols/4-program-connection.md) and [6. Checker](protocols/6-checker.md): the exits of [2. Log writer](protocols/2-log-writer.md) are rows of the first table, and those of [5. Relay response](protocols/5-relay-response.md) rows of the second. And one test per case of [3. Decide request](protocols/3-decide-request.md).
- Smuggling tests: single-request cases in the parser tables (L2), connection-level cases in `proxy`'s tests (L5). A second round is optional.

### Fake API

- Prints and records every request it receives, so tests can check what reached it and what didn't. Bytes that aren't a complete request (a head it can't parse, a request cut short, leftover bytes) are recorded too. A head it can't parse gets a `400` with `Connection: close` (RFC 9112 §2.2) then the connection closes in stages.
- Stops reading a connection once 2 MiB have arrived, before the reply and again after it. *Why:* that is more than the largest request the proxy forwards (an 8 KiB head and a 1 MiB body), so each one is recorded whole, while a sender that never stops can't fill the memory.
- Answers only the first request on each connection, then closes in stages (RFC 9112 §9.6). Anything sent after it is recorded but never answered, and every record carries the number of the connection it arrived on, so a proxy that sends two requests on one Server connection shows up in the tests (One exception is the response that never ends, it only sends, so nothing is read during it).
- Has endpoints with awkward responses: chunked, no length (ends when the connection closes), `204`, `/echo-headers` (sends back the request's head), a `103`, then a pause before the final response, a redirect to another host, and a response that never ends (it keeps sending until the proxy closes the connection).
- Has endpoints that misbehave, at least one for each reason a response can fail with: a `101`, a `1xx` with `Content-Length`, a head cut off by a close (all `bad_response`), silence before the head (`response_timeout`), and a body cut short (`response_incomplete`).

### Docker (at the end of v1)

- One image per program and a `compose.yaml`.
- The secret is a Compose secret given only to the proxy's container. It's a plain file mounted into the container, not encrypted.
- Two internal networks: `sandbox` (program and proxy) and `upstream` (proxy and fake API). The program isn't on `upstream`, so it can reach only the proxy.
- The log volume is mounted only in the proxy (read-write) and the checker (read-only).
- The other details are decided at that step.

### Claude skills

Each skill is created at the step where its task first appears. My first skill, written by hand to learn the format, lives outside this repo; Tally's skills are made with Claude's `skill-creator` to learn the tooling. Two of them are optional.

| Skill | Purpose | Step |
|---|---|---|
| `tally-review` | Review a finished step against this document and the protocols; explain problems instead of fixing them. Knows the non-goals, so it doesn't suggest them early | L0 |
| `tally-demo` | Run the end-to-end demo from a bundled script and read the log; later through `docker compose` | L6, L8 |
| `tally-parser-case` (optional) | Add a tricky HTTP input to the parser tables: exact bytes, expected outcome, why it matters | L2 |
| `tally-add-refusal-reason` (optional) | Checklist for adding a reason everywhere: the enum, this document's table, status code, log line, checker, tests, demo | L3 |

Each skill has 3 prompts that should trigger it and 2 that shouldn't, checked by hand.

## Build order

| Step | What |
|---|---|
| L0 | Design: this document, and a sequence diagram and pseudo code for each protocol. Setup: cargo workspace, `.gitignore`, my own `CLAUDE.md`, `git init`, skill `tally-review` |
| L1 | Fake API: prints and records each request; `Content-Length`, chunked, no-length, `204`, `103` and `/echo-headers` responses, a redirect, and the misbehaving endpoints |
| L2 | Request-head and response-head parsers, same rules, each with a table of good and tricky inputs; the response checks of step 2 of [5. Relay response](protocols/5-relay-response.md), and where a response's body ends |
| L3 | Loading and checking settings and secrets, with the settings hash (the checks of [1. Proxy startup](protocols/1-proxy-startup.md)), and decisions as pure functions ([3. Decide request](protocols/3-decide-request.md)) |
| L4 | Log writer ([2. Log writer](protocols/2-log-writer.md)): a new file per run, one writer, lines forced to disk, stop on error, `start` and `stop` lines and `seq`. The log format is frozen here |
| L5 | Proxy loop ([4. One program connection](protocols/4-program-connection.md) and [5. Relay response](protocols/5-relay-response.md), and the listen loop and shutdown of [1. Proxy startup](protocols/1-proxy-startup.md)): parse → decide → read body → log → rewrite → forward → relay. Connection-level smuggling tests written first: glued requests, lying lengths, leftover bytes. The biggest step |
| L6 | Fake program + end-to-end demo; skill `tally-demo` |
| L7 | Checker ([6. Checker](protocols/6-checker.md)) |
| L8 | Docker and Compose |
| Optional | Smuggling, round 2: more duplicate headers, disguised `Transfer-Encoding`, a server replying before the request body is sent. Skills `tally-parser-case` and `tally-add-refusal-reason` |

## Done when

- `cargo test` passes, including the parser tables and the smuggling tests.
- With `cargo` and with `docker compose up`:
  - a valid request reaches the fake API with the real key, and its `release` line is in the log. With `cargo`, a test also holds the Writer's answer and checks that nothing reaches the fake API until it's released;
  - a request without a placeholder to a listed host is forwarded and logged as `pass`; to any other host it's refused;
  - every refusal case can be triggered, is logged, and never reaches the fake API. That includes each `host_mismatch` case (disagreement, no `Host`, two `Host`) and each `malformed_request` rule;
  - a request with two keys, one allowed for the host and one not, sends nothing;
  - a placeholder outside its key's bound header reaches the fake API unchanged;
  - a redirect that takes the program's placeholder to another host is refused as `host_not_allowed`;
  - a log that fails on write gives `503` and stops the proxy.
- The checker flags a hand-edited illegal `release` line, a deleted line, and lines cut from the end of a run (exit 1), and reports a settings file that doesn't match the run as not judged (exit 2).
- In Compose, the program's container can't read the secrets file or the log, and can't reach the fake API directly.
- Each skill triggers on its prompts and does what its description says.

## Later versions

Ideas for next versions:
- Merkle tree: first a hash chain, then a Merkle tree (probably the one I am most interested in)
- Witnesses: signed snapshots of the log, checked and co-signed by a separate program
- TLS: the program speaks plain HTTP to the proxy on a local link; the proxy speaks HTTPS to the Server
- A real sandbox: the program's identity from a credential instead of a setting
- Performance: for example writing several waiting lines with one `sync_all` (can maybe serve as a good exercise?)
- Echoed keys: how to handle a server's response whose headers contain a real key?
