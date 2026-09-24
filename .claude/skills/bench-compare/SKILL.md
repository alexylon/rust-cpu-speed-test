---
name: bench-compare
description: Measure whether a change made the prime sieve benchmark faster or slower, by building two versions (a git ref and the working tree, or two refs) in release mode and running them in turn with a noise estimate. Use this whenever the user asks if something is faster, slower, a speedup or a regression, wants before/after numbers, or after changing the sieve, a storage variant, the run loop or the release profile, even if they only say "check performance" or "benchmark it". A single run of the app proves nothing, as runs vary by a few percent.
argument-hint: "[base ref] [new ref] [run flags]"
---

# Compare benchmark scores

Compare passes/s between two versions of the code. Arguments: $ARGUMENTS

1. Work out the two sides. The base defaults to `HEAD` and the new side to the working tree. To measure a commit that's already made, compare it with its parent, e.g. `abc123~1 abc123`; the script builds commits from copies, so never check anything out. If both sides have the same code, the result is only noise, so say that rather than reporting a change.
2. Tell the user this keeps every core busy for a few minutes (about 4 with the defaults), and that other load on the machine skews the numbers.
3. From the repo root, run the script with a 10-minute Bash timeout, or in the background for longer runs:

   ```bash
   .claude/skills/bench-compare/bench.sh [base ref] [new ref] [run flags]
   ```

   - Run flags go to both versions and default to all four variants (`--bytes --bits --bits-rotate --bits-striped`). Narrow them to the variants the change affects to save time, and leave out any variant that only one side has. Other flags such as `-l 10000000` work too.
   - The script sets the thread count and run length itself, so use environment variables instead of `-t` or `-s`: `THREADS` (default `"1 all"`, where `all` means every logical CPU), `SECS` per run (default 3) and `ROUNDS` (default 3). More rounds or seconds mean less noise but a longer wait.
4. Report the table. `change` compares the best runs, which is what the app shows as its score, and `mean chg` compares the averages. `spread` is the larger of the two sides' (max − min) / mean, a rough noise level. Call a change smaller than the spread noise, not a speedup or slowdown. If the script notes that both builds have the same machine code, every difference is noise, so lead with that. When a result is close, offer a longer run with more `ROUNDS` and say how long it would take, rather than starting one unasked. A warning about prime counts at the end means a bug, not a result.
