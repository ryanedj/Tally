---
name: tally-demo
description: Run Tally's end-to-end demo (the fake API, the proxy and the fake program from their binaries, then the proxy stopped by SIGTERM) and explain the run request by request against its log. Use it whenever the user asks to run the demo, to see Tally work with the real binaries, or what a real run's log looks like, even without the word "demo" (for example "does the whole thing work end to end now?" or "show me a release line from a real run"). Not for running the test suite, and not for reviewing code, which is tally-review.
argument-hint: "[what to look at in the run]"
context: fork
agent: general-purpose
background: false
allowed-tools: Bash(bash "${CLAUDE_SKILL_DIR}/scripts/demo.sh")
---

# Explain a run of the Tally demo

You are explaining one run of Tally's demo to the user, who designs and builds Tally to learn. The demo has already run, and its whole output is below. Explain what happened, and don't edit any file or fix anything you find: the user wants to understand each problem well enough to fix it themselves.

Request: $ARGUMENTS

## The run

!`bash "${CLAUDE_SKILL_DIR}/scripts/demo.sh"`

## What you are reading

- The fake program plays a sandboxed weather app. It holds placeholders (`TALLY_` and 32 hex characters) instead of its API keys, and sends 13 labelled requests through the proxy, one after another, each on its own connection. Lines starting with `>` are what it sent, lines starting with `<` what came back. It expects nothing: the log is the record.
- Request 8 gets a redirect to another host, and the program follows it with the same headers, as a compromised program would. That second hop is a request of its own, with its own log line.
- The fake API prints every request that reaches it. The real keys are in `demo/demo-secrets.toml` (fake values, committed for the demo), so you can tell a released key from a placeholder.
- The proxy's settings are `demo/settings.toml`.
- The log has one JSON line per decision, numbered by `seq`, opened by a `start` line and closed by a `stop` line. Since the requests run one at a time, their lines follow in the same order.
- The checker ran last, on the log and the settings. It prints one row per line, the counts, then its flags. Its exit code is 0 for a clean run, 1 if anything is flagged, and 2 if the run couldn't be judged. It sees only the log and the settings, so it can't tell what reached the fake API.

Before judging, read in `docs/DESIGN.md`: "The log" (the fields of each event), "Decisions and reasons" (each event and reason, and the reply it gets), "Handling one request" and "Checker". When several refusal reasons could apply, `docs/protocols/3-decide-request.md` says which wins.

## Report

Start with one line: whether the run went through (built, both servers started, the proxy exited with 0 after SIGTERM and logged its stop line), or which step failed. If a step failed, quote its output, say what it points to, and stop there.

Then one line per request, in order, the redirect's two hops on two lines. Each line gives what came back (the status and the body in a few words), the log line it got (event, keys, host, reason), and what reached the fake API: the real key, only a placeholder, a request with no key, or nothing. For example:

> 1. The weather call: 200 `ok`. `seq` 1, `release [weather_key] api.weather.local:80`. The fake API got the real key.

Then the start and stop lines, the proxy's exit code, and the checker's: its verdict, and each flag if it has any.

End with "Unexpected:", then each thing in the run that differs from DESIGN or from the request's label, a flag or a checker exit code other than 0 included: what you saw, the section of DESIGN it disagrees with, and what it could mean, without a fix. Or "nothing".

Write plainly: short sentences, no bold. Say when you're not sure.
