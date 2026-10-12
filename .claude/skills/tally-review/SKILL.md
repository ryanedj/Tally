---
name: tally-review
description: Review a finished Tally build step (L1 to L8) against docs/DESIGN.md and the protocols in docs/protocols/, and explain each problem without fixing it. Use it whenever the user says they finished a step or part of one, asks for a review of their uncommitted work, or asks whether their code matches the design or a protocol, even without the word "review" (for example "done with the log writer, does it hold up?" or "check my parser against protocol 4").
argument-hint: "[step, e.g. L2] [files to narrow the review]"
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

- The step: the one the request names (L1 to L8). If it names none, work it out from the changed files and say which step you assumed.
- The code: the changes above, and the new files listed by git status, read in full. If the request names files or folders, review only those.
- If nothing changed and no files are named, say so and stop.

## What to review against

Read these before the code, so you judge the code against the design and not the other way round:

1. The step's row in the build order of `docs/DESIGN.md`, and every section of DESIGN.md the row's topics touch.
2. The protocols the row names, in `docs/protocols/`: the pseudo code, its guarantees, and its exits table if it has one.
3. If the project's private instructions point to implementation notes, the section of those notes for this step.

## What to leave to the tools

The formatter and clippy have already judged layout and the lints set in the root `Cargo.toml`. Give their result in one line (clean, or how many problems and in which files) and don't repeat their findings. If clippy couldn't compile the code, say so: the rest of the review still holds, but the tools' verdict is missing.

## Non-goals, assumptions and later versions

DESIGN.md lists v1 non-goals, v1 assumptions and ideas for later versions. The user chose to leave these out of v1, so don't raise anything they cover as a problem. Watch for look-alikes: flooding the log with refusals is a non-goal, but a refusal that never reaches the log breaks a guarantee.

## Report

Start with the tools' line. Then up to three groups, most serious problem first in each. Anything that breaks a guarantee, such as a key leaving before its line is on disk, comes before the rest. Leave out empty groups.

1. Breaks the design: the code does something DESIGN.md or a protocol rules out, or misses something they require.
2. Code and design disagree: the code may be the better choice. Say why, and leave the decision to the user.
3. Missing tests: DESIGN.md asks for one test per row of the exits tables (protocols 1, 4 and 6) and one per case of protocol 3, and names other tests for some steps.

Write each problem like this, with no fixed code:

> `proxy/src/connection.rs:88`: the request is sent before the Writer answers.
> Protocol 4, step 4: the decision is logged and the proxy waits for the Writer's answer before step 5 connects.
> What goes wrong: if the proxy crashes after sending, the key has left with no line in the log.
> Where to look: protocol 4's `log_or_stop`, and what your code does with its answer.

"What goes wrong" is a concrete case: an input or an event, and the wrong outcome. If you can't name one, it's probably a matter of taste; leave it out. "Where to look" points to part of the design, a protocol step, or documentation (Rust, a crate, an RFC), never to the fix itself.

End with one line, "Left out on purpose:", listing what you noticed but didn't raise, each with the section of DESIGN.md it falls under, or "nothing".

Write plainly: short sentences, no bold. Say when you're not sure something is a problem, for example a library behaviour you couldn't check in its documentation.
