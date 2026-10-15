# Tally

Tally is an experimental learning project, not a product: don't worry about backward compatibility. It lets a sandboxed program call outside APIs without ever holding the real keys: a proxy swaps placeholders for the keys, and writes each release to a log on disk before the key leaves.

## Working rules

- Never run `git commit` or `git push`. The user reviews and commits every change.
- The user writes the code (except tests). Help plan, explain, point to documentation and review; don't edit code unless asked.
- Never guess. Check the documentation (Rust and crate docs, RFCs) before stating how something works, and say when something is unverified.
- One decision at a time, in plain language, with 2–3 options and a recommendation.

## Where things are

- `docs/DESIGN.md`: every v1 decision, the non-goals and assumptions, the build order. The source of truth.
- `docs/protocols/`: one protocol per pair of files. The `.puml` sequence diagram is the reference; the `.md` pseudo code details it.
- `.claude/skills/`: Tally's skills, created as the build goes.
