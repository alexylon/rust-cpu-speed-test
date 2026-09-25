use clap::builder::styling::{AnsiColor, Styles};
use clap::Parser;
use primes::{
    FlagStorage, FlagStorageBitVector, FlagStorageBitVectorRotate, FlagStorageBitVectorStriped,
    FlagStorageByteVector, PrimeSieve,
};
use std::{
    str::FromStr,
    thread,
    time::{Duration, Instant},
};
use ui::Ui;

mod ui;

pub mod primes {
    use std::collections::HashMap;

    /// Known prime counts, used to check results.
    pub struct PrimeValidator(HashMap<usize, usize>);
    impl Default for PrimeValidator {
        fn default() -> Self {
            // (limit, number of primes below it)
            let map = [
                (10, 4),
                (100, 25),
                (1000, 168),
                (10000, 1229),
                (100000, 9592),
                (1000000, 78498),
                (10000000, 664579),
                (100000000, 5761455),
            ]
            .iter()
            .copied()
            .collect();
            PrimeValidator(map)
        }
    }
    impl PrimeValidator {
        /// Whether `result` is the right count for `sieve_size`, or `None` if it isn't known.
        pub fn is_valid(&self, sieve_size: usize, result: usize) -> Option<bool> {
            if let Some(&expected) = self.0.get(&sieve_size) {
                Some(result == expected)
            } else {
                None
            }
        }

        #[allow(dead_code)]
        pub fn known_results(&self) -> &HashMap<usize, usize> {
            &self.0
        }
    }

    /// The sieve's true/false flags. Each variant stores them in a different way.
    pub trait FlagStorage {
        /// Creates `size` flags, all set to true.
        fn create_true(size: usize) -> Self;

        /// Clears every `skip`th flag, starting at `start`.
        fn reset_flags(&mut self, start: usize, skip: usize);

        fn get(&self, index: usize) -> bool;
    }

    /// One byte per flag.
    pub struct FlagStorageByteVector(Vec<u8>);
    impl FlagStorage for FlagStorageByteVector {
        fn create_true(size: usize) -> Self {
            FlagStorageByteVector(vec![1; size])
        }

        // the loop condition lets the compiler skip the bounds check on self.0[i]
        #[inline(always)]
        fn reset_flags(&mut self, start: usize, skip: usize) {
            let mut i = start;
            while i < self.0.len() {
                self.0[i] = 0;
                i += skip;
            }
        }

        fn get(&self, index: usize) -> bool {
            if let Some(val) = self.0.get(index) {
                *val == 1
            } else {
                false
            }
        }
    }

    /// One bit per flag, packed into 32-bit words. Builds a new bit mask for every flag it clears.
    pub struct FlagStorageBitVector {
        words: Vec<u32>,
        length_bits: usize,
    }

    const U32_BITS: usize = 32;
    impl FlagStorage for FlagStorageBitVector {
        fn create_true(size: usize) -> Self {
            let num_words = size / U32_BITS + (size % U32_BITS).min(1);
            FlagStorageBitVector {
                words: vec![0xffffffff; num_words],
                length_bits: size,
            }
        }

        #[inline(always)]
        fn reset_flags(&mut self, start: usize, skip: usize) {
            let mut i = start;
            while i < self.words.len() * U32_BITS {
                let word_idx = i / U32_BITS;
                let bit_idx = i % U32_BITS;
                // skips the bounds check, which the compiler doesn't reliably remove;
                // the loop condition keeps word_idx in range
                unsafe {
                    *self.words.get_unchecked_mut(word_idx) &= !(1 << bit_idx);
                }
                i += skip;
            }
        }

        fn get(&self, index: usize) -> bool {
            if index >= self.length_bits {
                return false;
            }
            let word = self.words.get(index / U32_BITS).unwrap();
            *word & (1 << (index % U32_BITS)) != 0
        }
    }

    /// Like `FlagStorageBitVector`, but rotates a single mask instead of building one per flag.
    pub struct FlagStorageBitVectorRotate {
        words: Vec<u32>,
        length_bits: usize,
    }

    impl FlagStorage for FlagStorageBitVectorRotate {
        fn create_true(size: usize) -> Self {
            let num_words = size / U32_BITS + (size % U32_BITS).min(1);
            FlagStorageBitVectorRotate {
                words: vec![0xffffffff; num_words],
                length_bits: size,
            }
        }

        #[inline(always)]
        fn reset_flags(&mut self, start: usize, skip: usize) {
            let mut i = start;
            let initial_bit_idx = start % U32_BITS;
            let mut rolling_mask: u32 = !(1 << initial_bit_idx);
            let roll_bits = skip as u32;
            while i < self.words.len() * U32_BITS {
                let word_idx = i / U32_BITS;
                // skips the bounds check, which the compiler doesn't reliably remove;
                // the loop condition keeps word_idx in range
                unsafe {
                    *self.words.get_unchecked_mut(word_idx) &= rolling_mask;
                }
                i += skip;
                rolling_mask = rolling_mask.rotate_left(roll_bits);
            }
        }

        fn get(&self, index: usize) -> bool {
            if index >= self.length_bits {
                return false;
            }
            let word = self.words.get(index / U32_BITS).unwrap();
            *word & (1 << (index % U32_BITS)) != 0
        }
    }

    const U8_BITS: usize = 8;

    /// One bit per flag, in bytes, filled in stripes: first bit 0 of every byte, then bit 1 of
    /// every byte, and so on. Clearing is fast while the sieve fits in the CPU cache, but slow
    /// once it doesn't.
    pub struct FlagStorageBitVectorStriped {
        words: Vec<u8>,
        length_bits: usize,
    }
    impl FlagStorageBitVectorStriped {
        fn ceiling(numerator: isize, denominator: isize) -> isize {
            (numerator + denominator - 1) / denominator
        }
    }
    impl FlagStorage for FlagStorageBitVectorStriped {
        fn create_true(size: usize) -> Self {
            let num_words = size / U8_BITS + (size % U8_BITS).min(1);
            Self {
                words: vec![0xff; num_words],
                length_bits: size,
            }
        }

        fn reset_flags(&mut self, start: usize, skip: usize) {
            let chunk = self.words.len();
            for bit in 0..8 {
                let mask = !(1_u8 << bit);

                // first word in this stripe that holds a flag to clear
                let chunk_start = bit * chunk;
                let earliest = start.max(chunk_start);
                let diff = earliest as isize - start as isize;
                let relative = Self::ceiling(diff, skip as isize) * skip as isize;
                let chunk_start = relative as usize + start - chunk_start;

                // a large skip can jump past a whole stripe
                if chunk_start < chunk {
                    let slice = &mut self.words[chunk_start..];
                    let mut i = 0;
                    while i < slice.len() {
                        slice[i] &= mask;
                        i += skip;
                    }
                }
            }
        }

        fn get(&self, index: usize) -> bool {
            if index > self.length_bits {
                return false;
            }
            let word_index = index % self.words.len();
            let bit_index = index / self.words.len();
            let word = self.words.get(word_index).unwrap();
            *word & (1 << bit_index) != 0
        }
    }

    /// The sieve itself, which works with any flag storage. It only stores odd numbers.
    pub struct PrimeSieve<T: FlagStorage> {
        sieve_size: usize,
        flags: T,
    }

    impl<T> PrimeSieve<T>
    where
        T: FlagStorage,
    {
        pub fn new(sieve_size: usize) -> Self {
            let num_flags = sieve_size / 2 + 1;
            PrimeSieve {
                sieve_size,
                flags: T::create_true(num_flags),
            }
        }

        fn is_num_flagged(&self, number: usize) -> bool {
            if number.is_multiple_of(2) {
                return false;
            }
            let index = number / 2;
            self.flags.get(index)
        }

        /// Counts the primes found. 1 is never cleared, so it is counted in place of 2,
        /// which isn't stored.
        pub fn count_primes(&self) -> usize {
            (1..self.sieve_size)
                .filter(|v| self.is_num_flagged(*v))
                .count()
        }

        /// Lists the primes found, adding 2 by hand, as only odd numbers are stored.
        pub fn primes(&self) -> Vec<usize> {
            std::iter::once(2)
                .chain((3..self.sieve_size).filter(|n| self.is_num_flagged(*n)))
                .collect()
        }

        /// Clears the flags of the odd numbers that aren't prime.
        pub fn run_sieve(&mut self) {
            let mut factor = 3;
            let q = (self.sieve_size as f32).sqrt() as usize;

            // include q itself, or limits like 1000 give the wrong count
            while factor <= q {
                factor = (factor..self.sieve_size)
                    .find(|n| self.is_num_flagged(*n))
                    .unwrap();

                // Clear the odd multiples of factor, starting at its square. Only odd numbers
                // are stored, so a step of `factor` flags is a step of 2 * factor in numbers.
                let start = factor * factor / 2;
                let skip = factor;
                self.flags.reset_flags(start, skip);

                factor += 2;
            }
        }
    }
}

/// Eratos measures CPU speed with a multi-threaded Sieve of Eratosthenes.
#[derive(Parser, Debug)]
#[command(version, styles = HELP_STYLES)]
struct CommandLineOptions {
    /// Number of threads [default: as many as the CPU can run at once]
    #[arg(short, long, value_parser = at_least_one::<usize>)]
    threads: Option<usize>,

    /// Run duration in seconds
    #[arg(short, long, default_value_t = 5, value_parser = at_least_one::<u64>)]
    seconds: u64,

    /// Count primes up to this number. Counts are checked against known results
    /// when it's a power of 10, up to 100000000
    #[arg(short, long, default_value_t = 1_000_000)]
    limit: usize,

    /// Number of runs of each variant
    #[arg(short, long, default_value_t = 1, value_parser = at_least_one::<usize>)]
    repetitions: usize,

    /// Print all primes found
    #[arg(short, long)]
    print: bool,

    /// Run variant that uses bit-level storage
    #[arg(long)]
    bits: bool,

    /// Run variant that uses bit-level storage with a rotating mask
    #[arg(long)]
    bits_rotate: bool,

    /// Run variant that uses striped bit-level storage (the default if no variant is selected)
    #[arg(long)]
    bits_striped: bool,

    /// Run variant that uses byte-level storage
    #[arg(long)]
    bytes: bool,
}

/// --help colours, like cargo's.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

/// Parses a whole number that is at least 1.
fn at_least_one<T>(arg: &str) -> Result<T, String>
where
    T: FromStr + PartialOrd + From<u8>,
    T::Err: std::fmt::Display,
{
    match arg.parse::<T>() {
        Ok(n) if n >= T::from(1) => Ok(n),
        Ok(_) => Err("must be at least 1".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

fn main() {
    let opt = CommandLineOptions::parse();

    let limit = opt.limit;
    let repetitions = opt.repetitions;
    let run_duration = Duration::from_secs(opt.seconds);
    let threads = opt.threads.unwrap_or_else(num_cpus::get);

    let ui = Ui::detect();
    ui.header(
        threads,
        opt.threads.is_none(),
        limit,
        run_duration,
        repetitions,
    );
    ui.table_header();

    let mut results = Vec::new();
    let mut run_variant = |label: &'static str, run: Runner| {
        // let the system settle before each variant, showing it as up next
        ui.progress(label, Duration::ZERO, run_duration);
        thread::sleep(Duration::from_secs(1));
        for _ in 0..repetitions {
            // every variant finds the same primes, so only the first run keeps them for --print
            let keep_primes = opt.print && results.is_empty();
            let result = run(label, run_duration, threads, limit, keep_primes, ui);
            ui.row(&result);
            results.push(result);
        }
    };

    // striped is the default when no variant is chosen
    let run_default = [opt.bits, opt.bits_rotate, opt.bits_striped, opt.bytes]
        .iter()
        .all(|b| !b);

    if opt.bytes {
        run_variant("byte-storage", run_implementation::<FlagStorageByteVector>);
    }

    if opt.bits {
        run_variant("bit-storage", run_implementation::<FlagStorageBitVector>);
    }

    if opt.bits_rotate {
        run_variant(
            "bit-storage-rotate",
            run_implementation::<FlagStorageBitVectorRotate>,
        );
    }

    if opt.bits_striped || run_default {
        run_variant(
            "bit-storage-striped",
            run_implementation::<FlagStorageBitVectorStriped>,
        );
    }

    ui.summary(&results);
    if opt.print {
        ui.primes(limit, &results[0].primes);
    }
}

/// The outcome of timing one variant.
struct RunResult {
    label: &'static str,
    threads: usize,
    /// sieve passes completed across all threads
    passes: usize,
    duration: Duration,
    /// primes counted by one of the sieves
    count: usize,
    /// whether `count` matches the known result, if there is one for this limit
    valid: Option<bool>,
    /// every prime found, only collected for --print
    primes: Vec<usize>,
}

impl RunResult {
    /// Sieve passes per second, across all threads.
    fn rate(&self) -> f64 {
        self.passes as f64 / self.duration.as_secs_f64()
    }

    /// Average time for one thread to complete a single pass, in seconds.
    fn pass_time(&self) -> f64 {
        self.duration.as_secs_f64() * self.threads as f64 / self.passes as f64
    }
}

/// `run_implementation` for one storage type.
type Runner = fn(&'static str, Duration, usize, usize, bool, Ui) -> RunResult;

/// Time one variant on `num_threads` threads for `run_duration`, showing a progress bar meanwhile.
fn run_implementation<T: 'static + FlagStorage + Send>(
    label: &'static str,
    run_duration: Duration,
    num_threads: usize,
    limit: usize,
    keep_primes: bool,
    ui: Ui,
) -> RunResult {
    // Each thread runs sieves until time is up, then returns its pass count and last sieve.
    // Returning the sieve stops the compiler from optimising the work away.
    let start_time = Instant::now();
    let threads: Vec<_> = (0..num_threads)
        .map(|_| {
            std::thread::spawn(move || {
                let mut local_passes = 0;
                let mut last_sieve = None;
                while (Instant::now() - start_time) < run_duration {
                    let mut sieve: PrimeSieve<T> = primes::PrimeSieve::new(limit);
                    sieve.run_sieve();
                    last_sieve.replace(sieve);
                    local_passes += 1;
                }
                (local_passes, last_sieve)
            })
        })
        .collect();

    // redraw the progress bar until time is up, waking on time so the end time stays accurate
    if ui.is_live() {
        while let Some(remaining) = run_duration.checked_sub(start_time.elapsed()) {
            ui.progress(label, start_time.elapsed(), run_duration);
            thread::sleep(remaining.min(Duration::from_millis(100)));
        }
        ui.progress(label, run_duration, run_duration);
    }

    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let end_time = Instant::now();
    ui.clear_progress();

    // all sieves are the same, so checking one is enough
    let passes = results.iter().map(|r| r.0).sum();
    let sieve = results.into_iter().next().and_then(|r| r.1);
    let count = sieve.as_ref().map_or(0, |sieve| sieve.count_primes());
    RunResult {
        label,
        threads: num_threads,
        passes,
        duration: end_time - start_time,
        count,
        valid: primes::PrimeValidator::default().is_valid(limit, count),
        primes: match sieve {
            Some(sieve) if keep_primes => sieve.primes(),
            _ => Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primes::PrimeValidator;

    #[test]
    fn sieve_known_correct_bits() {
        sieve_known_correct::<FlagStorageBitVector>();
    }

    #[test]
    fn sieve_known_correct_bits_rolling() {
        sieve_known_correct::<FlagStorageBitVectorRotate>();
    }

    #[test]
    fn sieve_known_correct_bits_striped() {
        sieve_known_correct::<FlagStorageBitVectorStriped>();
    }

    #[test]
    fn sieve_known_correct_bytes() {
        sieve_known_correct::<FlagStorageByteVector>();
    }

    fn sieve_known_correct<T: FlagStorage>() {
        let validator = PrimeValidator::default();
        for (sieve_size, expected_primes) in validator.known_results().iter() {
            let mut sieve: PrimeSieve<T> = primes::PrimeSieve::new(*sieve_size);
            sieve.run_sieve();
            assert_eq!(
                *expected_primes,
                sieve.count_primes(),
                "wrong number of primes for sieve = {}",
                sieve_size
            );
        }
    }
}
