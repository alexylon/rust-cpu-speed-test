use primes::{
    FlagStorage, FlagStorageBitVector, FlagStorageByteVector, FlagStorageBitVectorRotate,
    FlagStorageBitVectorStriped, PrimeSieve,
};
use std::{thread, time::{Duration, Instant}};
use structopt::clap::{AppSettings, Error, ErrorKind};
use structopt::StructOpt;
use ui::Ui;

mod ui;

pub mod primes {
    use std::{collections::HashMap, usize};

    /// Validator to compare against known primes.
    /// Pulled this out into a separate struct, as it's defined
    /// `const` in C++. There are various ways to do this in Rust, including
    /// lazy_static, etc. Should be able to do the const initialisation in the future.
    pub struct PrimeValidator(HashMap<usize, usize>);
    impl Default for PrimeValidator {
        fn default() -> Self {
            let map = [
                (10, 4),   // Historical data for validating our results - the number of primes
                (100, 25), // to be found under some limit, such as 168 primes under 1000
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
        // Return Some(true) or Some(false) if we know the answer, or None if we don't have
        // an entry for the given sieve_size.
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

    /// Trait defining the interface to different kinds of storage, e.g.
    /// bits within bytes, a vector of bytes, etc.
    pub trait FlagStorage {
        /// create new storage for given number of flags pre-initialised to all true
        fn create_true(size: usize) -> Self;

        /// reset all flags at indices starting at `start` with a stride of `stride`
        fn reset_flags(&mut self, start: usize, skip: usize);

        /// get a specific flag
        fn get(&self, index: usize) -> bool;
    }

    /// Storage using a simple vector of bytes.
    /// Doing the same with bools is equivalent, as bools are currently
    /// represented as bytes in Rust. However, this is not guaranteed to
    /// remain so for all time. To ensure consistent memory use in the future,
    /// we're explicitly using bytes (u8) here.
    pub struct FlagStorageByteVector(Vec<u8>);
    impl FlagStorage for FlagStorageByteVector {
        fn create_true(size: usize) -> Self {
            FlagStorageByteVector(vec![1; size])
        }


        // bounds checks are elided since we're runing up to .len()
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

    /// Storage using a vector of 32-bit words, but addressing individual bits within each. Bits are
    /// reset by applying a mask created by a shift on every iteration, similar to the C++ implementation.
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
                // Note: Unsafe usage to ensure that we elide the bounds check reliably.
                //       We have ensured that word_index < self.words.len().
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

    /// Storage using a vector of 32-bit words, but addressing individual bits within each. Bits are
    /// reset by rotating the mask left instead of modulo+shift.
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
                // Note: Unsafe usage to ensure that we elide the bounds check reliably.
                //       We have ensured that word_index < self.words.len().
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

    /// Storage using a vector of (8-bit) bytes, but individually addressing bits within
    /// each byte for bit-level storage. This is a fun variation I made up myself, but
    /// I'm pretty sure it's not original: someone must have done this before, and it
    /// probably has a name. If you happen to know, let me know :)
    ///
    /// The idea here is to store bits in a different order. First we make use of all the
    /// _first_ bits in each word. Then we come back to the start of the array and
    /// proceed to use the _second_ bit in each word, and so on.
    ///
    /// There is a computation / memory bandwidth tradeoff here. This works well
    /// only for sieves that fit inside the processor cache. For processors with
    /// smaller caches or larger sieves, this algorithm will result in a lot of
    /// cache thrashing.
    const U8_BITS: usize = 8;
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
                // get mask for this bit position
                let mask = !(1_u8 << bit);

                // calculate start word for this stripe
                let chunk_start = bit * chunk;
                let earliest = start.max(chunk_start);
                let diff = earliest as isize - start as isize;
                let relative = Self::ceiling(diff, skip as isize) * skip as isize;
                let chunk_start = relative as usize + start - chunk_start;

                // for larger `skips`, not every bit will have any corresponding words
                // take slice starting here, and reset the bit in every `skip`th word
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


    /// The actual sieve implementation, generic over the storage. This allows us to
    /// include the storage type we want without re-writing the algorithm each time.
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
            if number % 2 == 0 {
                return false;
            }
            let index = number / 2;
            self.flags.get(index)
        }

        // count number of primes (not optimal, but doesn't need to be)
        pub fn count_primes(&self) -> usize {
            (1..self.sieve_size)
                .filter(|v| self.is_num_flagged(*v))
                .count()
        }

        // list all primes found; 2 is implicit, as only odd numbers are stored
        pub fn primes(&self) -> Vec<usize> {
            std::iter::once(2)
                .chain((3..self.sieve_size).filter(|n| self.is_num_flagged(*n)))
                .collect()
        }

        // calculate the primes up to the specified limit
        pub fn run_sieve(&mut self) {
            let mut factor = 3;
            let q = (self.sieve_size as f32).sqrt() as usize;

            // note: need to check up to and including q, otherwise we
            // fail to catch cases like sieve_size = 1000
            while factor <= q {
                // find next factor - next still-flagged number
                factor = (factor..self.sieve_size)
                    .find(|n| self.is_num_flagged(*n))
                    .unwrap();

                // reset flags starting at `start`, every `factor`'th flag
                let start = factor * factor / 2;
                let skip = factor;
                self.flags.reset_flags(start, skip);

                factor += 2;
            }
        }
    }}

/// Measure CPU speed by counting primes with a multi-threaded sieve.
#[derive(StructOpt, Debug)]
#[structopt(setting = AppSettings::ColoredHelp)]
struct CommandLineOptions {
    /// Number of threads [default: all logical CPUs, including hyper-threads]
    #[structopt(short, long)]
    threads: Option<usize>,

    /// Run duration in seconds
    #[structopt(short, long, default_value = "5")]
    seconds: u64,

    /// Count primes up to this number. Counts are checked against known results
    /// when it's a power of 10, up to 100000000
    #[structopt(short, long, default_value = "1000000")]
    limit: usize,

    /// Number of runs of each variant
    #[structopt(short, long, default_value = "1")]
    repetitions: usize,

    /// Print all primes found
    #[structopt(short, long)]
    print: bool,

    /// Run variant that uses bit-level storage
    #[structopt(long)]
    bits: bool,

    /// Run variant that uses bit-level storage, applied using rotate
    #[structopt(long)]
    bits_rotate: bool,

    /// Run variant that uses bit-level storage, using striped storage
    /// (runs by default if no variant is selected)
    #[structopt(long)]
    bits_striped: bool,

    /// Run variant that uses byte-level storage
    #[structopt(long)]
    bytes: bool,
}

fn main() {
    // command line options are handled by the `structopt` and `clap` crates, which
    // makes life very pleasant indeed. At the cost of a bit of compile time :)
    let opt = CommandLineOptions::from_args();

    let limit = opt.limit;
    let repetitions = opt.repetitions;
    let run_duration = Duration::from_secs(opt.seconds);

    // all logical CPUs (including hyper-threads / vCPUs), unless --threads is given
    let threads = opt.threads.unwrap_or_else(num_cpus::get);

    // reject settings that would leave nothing to measure
    for (flag, value) in [
        ("--threads", threads as u64),
        ("--seconds", opt.seconds),
        ("--repetitions", repetitions as u64),
    ] {
        if value == 0 {
            Error::with_description(&format!("{} must be at least 1", flag), ErrorKind::InvalidValue)
                .exit();
        }
    }

    let ui = Ui::detect();
    ui.header(threads, opt.threads.is_none(), limit, run_duration, repetitions);
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

    // run only the striped implementation if no variant is specified (default)
    let run_default = [opt.bits, opt.bits_rotate, opt.bits_striped, opt.bytes].iter().all(|b| !b);

    if opt.bytes {
        run_variant("byte-storage", run_implementation::<FlagStorageByteVector>);
    }

    if opt.bits {
        run_variant("bit-storage", run_implementation::<FlagStorageBitVector>);
    }

    if opt.bits_rotate {
        run_variant("bit-storage-rotate", run_implementation::<FlagStorageBitVectorRotate>);
    }

    if opt.bits_striped || run_default {
        run_variant("bit-storage-striped", run_implementation::<FlagStorageBitVectorStriped>);
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
    /// every prime found, only collected when they're going to be printed
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
    // spin up N threads; each will terminate itself after `run_duration`, returning
    // the last sieve as well as the total number of counts.
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
                // return local pass count and last sieve
                (local_passes, last_sieve)
            })
        })
        .collect();

    // animate the progress bar until the threads are due to stop, waking up
    // on time so that the end time recorded below stays accurate
    if ui.is_live() {
        while let Some(remaining) = run_duration.checked_sub(start_time.elapsed()) {
            ui.progress(label, start_time.elapsed(), run_duration);
            thread::sleep(remaining.min(Duration::from_millis(100)));
        }
        ui.progress(label, run_duration, run_duration);
    }

    // wait for threads to finish, and record end time
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let end_time = Instant::now();
    ui.clear_progress();

    // get totals, and check the primes counted by one of the sieves
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
