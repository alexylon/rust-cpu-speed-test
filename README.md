# Eratos

Eratos is a command-line CPU benchmark written in Rust. It repeatedly finds prime numbers using the Sieve of Eratosthenes and reports how many complete passes it runs per second.

## Build and run

Install [Rust and Cargo](https://rust-lang.org/tools/install/) if needed, then run these commands from the project directory:

```sh
cargo build --release
./target/release/eratos
```

On Windows, run `.\target\release\eratos.exe` instead. Use a release build for benchmarking; debug builds are much slower.

The default test runs for five seconds, finding primes below 1,000,000. It uses all available CPU threads and the striped storage variant. Allow about six seconds in total, including a one-second pause before the test.

## Choose a test

```sh
# Use one CPU thread
./target/release/eratos --threads 1

# Run three 10-second tests with four threads
./target/release/eratos --threads 4 --seconds 10 --repetitions 3

# Show all options
./target/release/eratos --help
```

| Option | What it changes | Default |
| --- | --- | --- |
| `--threads N` | Number of worker threads. | All available CPU threads |
| `--seconds N` | Duration of each timed run, in seconds. | `5` |
| `--repetitions N` | Number of runs for each selected variant. | `1` |
| `--limit N` | Find primes below this number. | `1000000` |
| `--print` | Print the primes after the results. | Off |

Eratos includes four variants that store the sieve's working data differently. With no variant flags, it runs `--bits-striped`. To run all four:

```sh
./target/release/eratos --bytes --bits --bits-rotate --bits-striped
```

You can select any combination of those flags. Each selected variant runs for the chosen duration and number of repetitions, with a one-second pause before its first run.

## Read the results

Each row describes one run:

| Column | Meaning |
| --- | --- |
| `PASSES` | Complete sieve passes across all threads. |
| `PASSES/S` | Passes per second across all threads. Higher is faster. |
| `PASS TIME` | Estimated average time for one thread to complete a pass. Lower is faster. |
| `PRIMES` | Number of primes found in a single pass. |

The **Score** is the highest `PASSES/S` result. If you repeat the test or select multiple variants, it is the best result across all of those runs.

The symbol beside the prime count shows whether it matches a known result:

- `✓` — the count is correct.
- `✗` — the count is wrong; the result should not be used.
- `?` — there is no reference count for that limit, so the result is unchecked.

Reference counts are available for powers of 10 from `10` through `100000000`.

For comparisons, use the same Eratos version, variant, prime limit, and duration, and record the thread count. Repeat the test and avoid other CPU-heavy work while it runs. Small differences can be normal variation between runs.

## Credits

The sieve and storage variants are based on the July 2021 version of the [Rust solution](https://github.com/PlummersSoftwareLLC/Primes/tree/9c6df85caa5079eb238f7ed9d264a05c1579395e/PrimeRust/solution_1) by Michael Barber (@mike-barber), written for Dave Plummer's [Primes: A Software Drag Race](https://github.com/PlummersSoftwareLLC/Primes).

## License

Licensed under the [BSD 3-Clause License](LICENSE), the same license used by the original drag race solutions.
