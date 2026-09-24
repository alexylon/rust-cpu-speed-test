# rust-cpu-speed-test

Test CPU speed by calculating prime numbers. By default it runs as many threads as the CPU can run at once (one per core on Apple Silicon, one per vCPU on a cloud VM); pass `--threads N` to override.

`cargo build --release`

## Credits

The sieve and its storage variants come from the [Rust solution](https://github.com/PlummersSoftwareLLC/Primes/tree/9c6df85caa5079eb238f7ed9d264a05c1579395e/PrimeRust/solution_1) by Michael Barber (@mike-barber) for Dave Plummer's [Primes: A Software Drag Race](https://github.com/PlummersSoftwareLLC/Primes), as it was in July 2021.

## License

BSD 3-Clause, as for the drag race's solutions. See [LICENSE](LICENSE).
