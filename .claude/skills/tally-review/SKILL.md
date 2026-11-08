---
name: tally-review
description: Review a finished Tally build step (L1 to L8) against docs/DESIGN.md and the protocols in docs/protocols/, check its claims against their saved sources, and explain each problem without fixing it. Use it whenever the user says they finished a step or part of one, asks for a review of their uncommitted work, asks whether their code matches the design or a protocol, or asks whether the project's claims and sources hold, even without the word "review" (for example "done with the log writer, does it hold up?", "check my parser against protocol 4" or "are the quotes in my notes right?").
argument-hint: "[step, e.g. L2] [claims, to check every claim] [files to narrow the review]"
context: fork
agent: general-purpose
background: false
allowed-tools: Bash(bash "${CLAUDE_SKILL_DIR}/checks.sh")
---

# Review a Tally step

You are reviewing a build step of Tally that the user wrote by hand. They are learning: they write all the code, and they want to understand each problem well enough to fix it themselves. So explain, and don't edit any file or write fixed code.

Request: $ARGUMENTS

## Facts gathered before you started

git, the formatter and the linter have already run. Their output, with each exit code:

!`bash "${CLAUDE_SKILL_DIR}/checks.sh"`

## What to review

- If the request says `claims`, review no code: do only "Check the claims" below, and report only that.
- The step: the one the request names (L1 to L8). If it names none, work it out from the changed files and say which step you assumed.
- The code: the changes above, and the new files listed by git status, read in full. If the request names files or folders, review only those.
- If nothing changed and no files are named, say so and stop.

## What to review against

Read these before the code, so you judge the code against the design and not the other way round:

1. The step's row in the build order of `docs/DESIGN.md`, and every section of DESIGN.md the row's topics touch.
2. The protocols the row names, in `docs/protocols/`: the pseudo code, its guarantees, and its exits table if it has one.
3. If the project's private instructions point to implementation notes, the section of those notes for this step.

## Check the claims

The design is the reference for the code, not for itself. So here, don't take DESIGN.md or the notes as true: check what they say against their sources.

A claim says what a project, paper, standard or tool does or says, or what a run or a measurement showed. This project's own decisions aren't claims.

`notes/` is git-ignored, so it's in no diff, and a fresh clone doesn't have it. If `notes/SOURCES.md` is missing, say so in the claims line and skip this section.

Which claims:

- By default, those the step added or changed: the changed lines of `docs/`, `README.md`, `.claude/skills/` and code comments, and the step's sections of `notes/IMPLEMENTATION.md` and `notes/SOURCES.md`, read by name. `notes/SOURCES.md` has a section per step only from L6 on, so an earlier step may have none there.
- If the request says `claims`, every claim in those files, and in all of `notes/IMPLEMENTATION.md` and `notes/SOURCES.md`. No other file in `notes/` is in scope.
- If the request names files or folders, only the claims in those.

For each claim:

0. Split the claim into atomic facts: each number, name, section, quote and qualifier. A sentence that is mostly true can hold one wrong detail ("retried after 1 s", true except for one kind of error; "a 400 with `Connection: close` (RFC 9112 §2.2)", where §2.2 gives the 400 and not the header), and judging it whole hides that. Check each fact below, and count facts, not sentences. The verdict names the fact that fails.
1. Find its entry in `notes/SOURCES.md`, then the saved file in `notes/sources/`: the header of `notes/SOURCES.md` says how a link gives a file's name. Read the cited lines in that file, not the quote in the notes. Line numbers such as `DESIGN:49` are those of the file when it was checked and have moved, so find a claim by its words.
2. Compare them with everything the claim says. A quote that exists isn't enough: the source must say it explicitly. A claim that names three projects needs all three.
3. For a run or a measurement, read the store or output it names, if you can reach it, such as a demo run's log in `demo/logs/`, or a saved run in `notes/runs/` (its command first, then its output). Some runs left nothing to read, such as a check made by breaking the code on purpose. Give those one Not found between them, so they don't bury the rest.
4. Give a verdict, in the notes' words: Confirmed, Partly (say which part holds), Wrong, or Not found (no entry, no saved file, or not in the cited lines; say where you looked).

A claim about the present, such as "the latest release" or "the most used", can go stale on its own, so it needs a date in the claim or its entry. Without one it's Partly.

A claim that something doesn't exist, or that nobody does something, rests on a search. It needs a control: the same search finding something it should, recorded with the empty result. Without one it's Not found, since an empty result may only mean the search was broken.

Don't fetch anything from the web. A source that isn't saved is Not found, and saving it isn't part of the review.

### A full audit

When the request says `claims` and names no files, there are a few hundred claims, too many to read with care in one pass: a narrower check once found five that a single full pass had missed. So split the work:

1. Cut the files into parts of about 20 to 40 claims, each made of whole sections: `docs/DESIGN.md` by its top-level sections, grouped where they're short; `docs/protocols/` as one part; `README.md`, `.claude/skills/` and the code comments as one; `notes/IMPLEMENTATION.md` by build step; `notes/SOURCES.md` by section. Parts don't overlap: give each a file and its line range, and list the parts in your report, so no claim is counted twice.
2. Start one subagent per part with the Agent tool, at most 10 running at a time. Give each its part, the rules of this section from "The design is the reference" to "Don't fetch", word for word, and the claim format from the Report below. Tell it to edit nothing and fetch nothing, and to return its counts of atomic facts and each fact that isn't Confirmed.
3. Merge them: one claims line with the sums, then group 3, each finding labelled with its part. Read the cited lines yourself for every finding that is Wrong or Partly, since a part's reviewer can be wrong too. Findings that are only a missing source or run need no reading.

## What to leave to the tools

The formatter and clippy have already judged layout and the lints set in the root `Cargo.toml`. Give their result in one line (clean, or how many problems and in which files) and don't repeat their findings. If clippy couldn't compile the code, say so: the rest of the review still holds, but the tools' verdict is missing.

## Non-goals, assumptions and later versions

DESIGN.md lists v1 non-goals, v1 assumptions and ideas for later versions. The user chose to leave these out of v1, so don't raise anything they cover as a problem. Watch for look-alikes: flooding the log with refusals is a non-goal, but a refusal that never reaches the log breaks a guarantee.

## Report

Start with the tools' line, then one line counting the claims, such as `Claims: 12 checked, 10 confirmed, 1 partly, 0 wrong, 1 not found.` Then up to four groups, most serious problem first in each. Anything that breaks a guarantee, such as a key leaving before its line is on disk, comes before the rest. Leave out empty groups. If the request says `claims`, give only the claims line and group 3.

1. Breaks the design: the code does something DESIGN.md or a protocol rules out, or misses something they require.
2. Code and design disagree: the code may be the better choice. Say why, and leave the decision to the user.
3. Claims and sources: each claim that isn't Confirmed. Confirmed claims are only counted, never listed.
4. Missing tests: DESIGN.md asks for one test per row of the exits tables (protocols 1, 4 and 6) and one per case of protocol 3, and names other tests for some steps.

Write each problem like this, with no fixed code:

> `proxy/src/connection.rs:88`: the request is sent before the Writer answers.
> Protocol 4, step 4: the decision is logged and the proxy waits for the Writer's answer before step 5 connects.
> What goes wrong: if the proxy crashes after sending, the key has left with no line in the log.
> Where to look: protocol 4's `log_or_stop`, and what your code does with its answer.

Write each claim that isn't Confirmed like this:

> `docs/DESIGN.md:49`: "nginx and Envoy both limit a body to 1 MiB".
> Verdict: Partly. nginx does (`nginx-nginx.org-159f0a3-xml-en-docs-http-ngx_http_core_module.xml` L511). `notes/SOURCES.md` has no entry for Envoy's limit.
> Looked in: `notes/SOURCES.md`, and the Envoy files in `notes/sources/`.

"What goes wrong" is a concrete case: an input or an event, and the wrong outcome. If you can't name one, it's probably a matter of taste; leave it out. "Where to look" points to part of the design, a protocol step, or documentation (Rust, a crate, an RFC), never to the fix itself.

End with one line, "Left out on purpose:", listing what you noticed but didn't raise, each with the section of DESIGN.md it falls under, or "nothing".

Write plainly: short sentences, no bold. Say when you're not sure something is a problem, for example a library behaviour you couldn't check in its documentation.
