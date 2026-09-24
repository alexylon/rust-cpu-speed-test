# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

A CPU benchmark: each thread runs a prime sieve over and over for a fixed time, and the score is total sieve passes per second.

## Commands

- The binary is `prime_race_rust`, not the repo name: `cargo run --release -- <flags>`.
- Only `--release` numbers mean anything; debug builds run ~6x slower.
- A default run pins every logical core for ~6 s (1 s settle + 5 s) and runs only the striped variant.
- Quick check of all four storage variants: `cargo run --release -- -s 1 -t 1 --bytes --bits --bits-rotate --bits-striped`. Every row should end in ✓.
- Before finishing a change, run `cargo test`, `cargo clippy --all-targets` and that quick check.

## Measuring performance

- Scores don't need to stay comparable across versions: faster code or new defaults are fine as long as prime counts stay correct.
- Measure changes with `/bench-compare`. Runs vary by a few percent and the score is the best run, not the mean, so a single run proves nothing.
- The sieve work survives optimization only because each worker returns its last sieve and thread 0's prime count is validated after timing. When restructuring the run loop, keep the result observable (or use `std::hint::black_box`), or the optimizer may delete the work.

## Gotchas

- Adding a storage variant takes 5 edits in `src/main.rs`: a `FlagStorage` impl, a bool CLI flag, the `run_default` array, the dispatch `if` in `main`, and a test. Keep its label within `LABEL_WIDTH` (21) characters, since the table and progress bar are sized from it.
- `--limit` is exclusive (primes below N), and prime counts are only validated when it's a power of 10 up to 1e8.
- Results go to stdout and the progress bar to stderr (TTY only). Format widths count ANSI escape bytes, so pad first and then wrap in `Paint`, as in `p.bold(&format!("{:>13}", …))`.
- Print results with the `out!` macro in `src/ui.rs`, not `println!`, so the program exits cleanly when output is piped into `head`.
- Comments: short, clear and in plain English, without jargon. Don't explain what the code already says. British spelling (colours, optimising).

## Git

- Commit only when asked: a one-line plain commit message, no Co-Authored-By trailer, and never push.
