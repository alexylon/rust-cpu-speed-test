//! Terminal output: a header describing the machine and the run, a live
//! progress bar while each variant runs, then a results table and a score.
//!
//! Results go to stdout and the progress bar to stderr. Colours and the
//! progress bar are only used on a terminal, and `NO_COLOR` turns colours off.

use std::collections::BTreeSet;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process;
use std::time::Duration;

use crate::RunResult;

/// Width of the first column, holding variant names in the table and progress bar.
const LABEL_WIDTH: usize = 21;
const BAR_WIDTH: usize = 30;
/// Covers the longest progress line, so that it can be erased.
const PROGRESS_WIDTH: usize = 2 + LABEL_WIDTH + BAR_WIDTH + 18;

/// Like `println!`, but ends the program instead of panicking when stdout is closed.
macro_rules! out {
    ($($arg:tt)*) => {
        if let Err(err) = writeln!(io::stdout(), $($arg)*) {
            stdout_failed(err)
        }
    };
}

/// Ends the program when results can't be written. A closed pipe, as with `| head`,
/// just means nobody is reading any more, so that isn't an error.
fn stdout_failed(err: io::Error) -> ! {
    if err.kind() == io::ErrorKind::BrokenPipe {
        process::exit(0);
    }
    eprintln!("error: can't write results: {}", err);
    process::exit(1);
}

/// Where output goes and how it's styled, detected once at startup.
#[derive(Clone, Copy)]
pub struct Ui {
    /// styling for stdout, which gets the results
    out: Paint,
    /// styling for stderr, which gets the progress bar
    err: Paint,
    /// whether stderr can show a live progress bar
    live: bool,
}

impl Ui {
    pub fn detect() -> Self {
        let stderr_is_terminal = io::stderr().is_terminal();
        Ui {
            out: Paint::for_stream(io::stdout().is_terminal()),
            err: Paint::for_stream(stderr_is_terminal),
            live: stderr_is_terminal && !is_dumb_terminal(),
        }
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Title, followed by the machine and the benchmark settings.
    pub fn header(
        &self,
        threads: usize,
        auto_threads: bool,
        limit: usize,
        run_duration: Duration,
        repetitions: usize,
    ) {
        let p = self.out;
        let field =
            |name: &str, value: String| out!("  {}  {}", p.dim(&format!("{:<8}", name)), value);

        out!();
        out!("  {}  {}", p.bold("Eratos"), p.dim("prime sieve benchmark"));
        out!();
        field(
            "CPU",
            cpu_model().unwrap_or_else(|| env::consts::ARCH.to_string()),
        );
        let logical = num_cpus::get();
        let physical = num_cpus::get_physical();
        field("Cores", describe_cores(logical, physical, core_types()));
        field(
            "Threads",
            if auto_threads {
                let per = if logical > physical {
                    "CPU thread"
                } else {
                    "core"
                };
                format!("{}{}", threads, p.dim(&format!(", one per {}", per)))
            } else {
                threads.to_string()
            },
        );
        field("Sieve", format!("primes up to {}", group(limit)));
        field("Duration", format!("{} s per run", run_duration.as_secs()));
        if repetitions > 1 {
            field("Runs", format!("{} per variant", repetitions));
        }
        if cfg!(debug_assertions) {
            field(
                "Build",
                p.yellow("debug, so expect far lower numbers than with --release"),
            );
        }
        out!();
    }

    /// Column headings for the results table.
    pub fn table_header(&self) {
        let headings = format!(
            "{:<w$}{:>13}{:>13}{:>12}{:>11}",
            "VARIANT",
            "PASSES",
            "PASSES/S",
            "PASS TIME",
            "PRIMES",
            w = LABEL_WIDTH
        );
        out!("  {}", self.out.dim(&headings));
    }

    /// A finished run, as a row of the results table.
    pub fn row(&self, run: &RunResult) {
        let p = self.out;
        let check = match run.valid {
            Some(true) => p.green("✓"),
            Some(false) => p.red("✗"),
            None => p.yellow("?"),
        };
        out!(
            "  {:<w$}{:>13}{}{:>12}{:>11} {}",
            run.label,
            group(run.passes),
            p.bold(&format!("{:>13}", group(run.rate().round() as usize))),
            human_time(run.pass_time()),
            group(run.count),
            check,
            w = LABEL_WIDTH
        );
    }

    /// Draw the progress bar of a variant that has run for `elapsed` out of `total`.
    pub fn progress(&self, label: &str, elapsed: Duration, total: Duration) {
        if !self.live {
            return;
        }
        let p = self.err;
        let elapsed = elapsed.min(total);
        let filled =
            (BAR_WIDTH as f64 * elapsed.as_secs_f64() / total.as_secs_f64()).round() as usize;
        let line = format!(
            "\r  {:<w$}{}{}  {}",
            label,
            p.cyan(&"━".repeat(filled)),
            p.dim(&"─".repeat(BAR_WIDTH - filled)),
            p.dim(&format!(
                "{:.1} / {} s",
                elapsed.as_secs_f64(),
                total.as_secs()
            )),
            w = LABEL_WIDTH
        );
        // a single write per frame, so the line is never seen half-drawn
        let _ = io::stderr().write_all(line.as_bytes());
    }

    /// Erase the progress bar, leaving the cursor at the start of its line.
    pub fn clear_progress(&self) {
        if self.live {
            let _ = write!(io::stderr(), "\r{:w$}\r", "", w = PROGRESS_WIDTH);
        }
    }

    /// The fastest run as a headline score, plus notes on any unchecked or wrong counts.
    pub fn summary(&self, runs: &[RunResult]) {
        let p = self.out;
        let Some(best) = runs.iter().max_by(|a, b| a.rate().total_cmp(&b.rate())) else {
            return;
        };
        let per_thread = best.rate() / best.threads as f64;
        let mut detail = format!("{} per thread", group(per_thread.round() as usize));
        if runs.iter().any(|run| run.label != best.label) {
            detail = format!("{} · {}", best.label, detail);
        }
        out!();
        out!(
            "  {}  {}  {}",
            p.bold("Score"),
            p.green(&p.bold(&format!("{} passes/s", group(best.rate().round() as usize)))),
            p.dim(&format!("({})", detail))
        );
        if runs.iter().any(|run| run.valid == Some(false)) {
            out!(
                "  {}",
                p.red("✗ some runs counted the wrong number of primes")
            );
        }
        if runs.iter().any(|run| run.valid.is_none()) {
            out!(
                "  {}",
                p.yellow("? prime counts are only checked when the limit is a power of 10")
            );
        }
        out!();
    }

    /// Every prime found, in right-aligned columns.
    pub fn primes(&self, limit: usize, primes: &[usize]) {
        let mut out = io::BufWriter::new(io::stdout().lock());
        if let Err(err) = self.write_primes(&mut out, limit, primes) {
            stdout_failed(err);
        }
    }

    fn write_primes(&self, out: &mut impl Write, limit: usize, primes: &[usize]) -> io::Result<()> {
        let title = format!("{} primes up to {}", group(primes.len()), group(limit));
        writeln!(out, "  {}", self.out.bold(&title))?;
        // two spaces between columns, which also indent the first one
        let width = primes.last().map_or(1, |prime| prime.to_string().len()) + 2;
        for line in primes.chunks((78 / width).clamp(1, 10)) {
            for prime in line {
                write!(out, "{:>w$}", prime, w = width)?;
            }
            writeln!(out)?;
        }
        writeln!(out)?;
        out.flush()
    }
}

/// Colours and bold text for one output stream. Text is left as is when colours are off.
#[derive(Clone, Copy)]
struct Paint(bool);

impl Paint {
    fn for_stream(is_terminal: bool) -> Self {
        let no_color = env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        // the old Windows console can't show colours and prints the codes as text
        let legacy_console =
            cfg!(windows) && env::var_os("WT_SESSION").is_none() && env::var_os("TERM").is_none();
        Paint(is_terminal && !no_color && !legacy_console && !is_dumb_terminal())
    }

    fn wrap(self, code: &str, text: &str) -> String {
        if self.0 {
            format!("\x1b[{}m{}\x1b[0m", code, text)
        } else {
            text.to_string()
        }
    }

    fn bold(self, text: &str) -> String {
        self.wrap("1", text)
    }

    fn dim(self, text: &str) -> String {
        self.wrap("2", text)
    }

    fn red(self, text: &str) -> String {
        self.wrap("31", text)
    }

    fn green(self, text: &str) -> String {
        self.wrap("32", text)
    }

    fn yellow(self, text: &str) -> String {
        self.wrap("33", text)
    }

    fn cyan(self, text: &str) -> String {
        self.wrap("36", text)
    }
}

fn is_dumb_terminal() -> bool {
    env::var("TERM").is_ok_and(|term| term == "dumb")
}

/// An integer with thousands separators, e.g. 1234567 -> "1,234,567".
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::with_capacity(digits.len() * 4 / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Seconds as a short duration with three significant digits, e.g. "442 µs".
fn human_time(secs: f64) -> String {
    let (value, unit) = match secs {
        s if s >= 1.0 => (s, "s"),
        s if s >= 1e-3 => (s * 1e3, "ms"),
        s if s >= 1e-6 => (s * 1e6, "µs"),
        s => (s * 1e9, "ns"),
    };
    let decimals = match value {
        v if v >= 100.0 => 0,
        v if v >= 10.0 => 1,
        _ => 2,
    };
    format!("{:.*} {}", decimals, value, unit)
}

/// The CPU's model name, where the OS makes it easy to find.
fn cpu_model() -> Option<String> {
    let name = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()?
            .lines()
            .filter_map(|line| line.split_once(':'))
            // x86 calls it "model name"; some ARM boards (e.g. Raspberry Pi) only have "Model"
            .find(|(key, _)| matches!(key.trim(), "model name" | "Model"))
            .map(|(_, value)| value.to_string())?
    } else if cfg!(target_os = "macos") {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()?;
        String::from_utf8(output.stdout).ok()?
    } else {
        return None;
    };
    // some CPUs pad their names with runs of spaces
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(name)
}

/// The core count in plain words, e.g. "8 (16 threads)" or "11 (5 performance, 6 efficiency)".
fn describe_cores(logical: usize, physical: usize, types: Option<(usize, usize)>) -> String {
    // fewer CPUs than cores means the OS or a container limits what this program can use
    let mut text = if logical < physical {
        format!("{} available, of {}", logical, physical)
    } else {
        physical.to_string()
    };
    let mut details = Vec::new();
    if logical > physical {
        details.push(format!("{} threads", logical));
    }
    // a split that doesn't add up means an unexpected layout, so leave it out
    if let Some((performance, efficiency)) = types.filter(|&(p, e)| p + e == physical) {
        details.push(format!(
            "{} performance, {} efficiency",
            performance, efficiency
        ));
    }
    if !details.is_empty() {
        text = format!("{} ({})", text, details.join(", "));
    }
    text
}

/// How many performance and efficiency cores the CPU has. Macs report this, and so does
/// Linux for Intel chips with both kinds.
fn core_types() -> Option<(usize, usize)> {
    if cfg!(target_os = "linux") {
        return linux_core_types(Path::new("/sys"));
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("sysctl")
        .args([
            "-n",
            "hw.nperflevels",
            "hw.perflevel0.physicalcpu",
            "hw.perflevel1.physicalcpu",
        ])
        .output()
        .ok()?;
    let counts: Vec<usize> = String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .map(|line| line.trim().parse().ok())
        .collect::<Option<_>>()?;
    match counts[..] {
        [2, performance, efficiency] => Some((performance, efficiency)),
        _ => None,
    }
}

/// Performance and efficiency core counts from Linux's `/sys` folder (`sys`). On Intel chips
/// with both kinds, the kernel lists their CPUs in `devices/cpu_core/cpus` and
/// `devices/cpu_atom/cpus`, plus `devices/cpu_lowpower/cpus` for the low-power efficiency
/// cores that some laptop chips have.
fn linux_core_types(sys: &Path) -> Option<(usize, usize)> {
    let cores = |kind: &str| -> Option<usize> {
        let cpus = std::fs::read_to_string(sys.join(format!("devices/{}/cpus", kind))).ok()?;
        // both threads of a hyper-threaded core list the same siblings, so each core counts once
        let siblings: BTreeSet<String> = parse_cpu_list(&cpus)?
            .into_iter()
            .map(|cpu| {
                let path = format!(
                    "devices/system/cpu/cpu{}/topology/thread_siblings_list",
                    cpu
                );
                let list = std::fs::read_to_string(sys.join(path)).ok()?;
                Some(list.trim().to_string())
            })
            .collect::<Option<_>>()?;
        Some(siblings.len())
    };
    let low_power = cores("cpu_lowpower").unwrap_or(0);
    Some((cores("cpu_core")?, cores("cpu_atom")? + low_power))
}

/// Expands a Linux CPU list such as "0-3,8" into [0, 1, 2, 3, 8].
fn parse_cpu_list(list: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for part in list.trim().split(',') {
        match part.split_once('-') {
            Some((first, last)) => cpus.extend(first.parse::<usize>().ok()?..=last.parse().ok()?),
            None => cpus.push(part.parse().ok()?),
        }
    }
    Some(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_cores_reads_plainly() {
        assert_eq!(describe_cores(4, 4, None), "4");
        assert_eq!(describe_cores(16, 8, None), "8 (16 threads)");
        assert_eq!(
            describe_cores(11, 11, Some((5, 6))),
            "11 (5 performance, 6 efficiency)"
        );
        assert_eq!(describe_cores(2, 8, None), "2 available, of 8");
        assert_eq!(describe_cores(16, 16, Some((6, 8))), "16");
    }

    #[test]
    fn parse_cpu_list_expands_ranges() {
        assert_eq!(parse_cpu_list("0-3,8\n"), Some(vec![0, 1, 2, 3, 8]));
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(parse_cpu_list(""), None);
    }

    #[test]
    fn linux_core_types_count_cores_not_threads() {
        // Core Ultra 7 265T: 8 performance and 12 efficiency cores, no hyper-threading
        let sys = fake_sys(
            "arrow-lake",
            &[("cpu_core", "0-7"), ("cpu_atom", "8-19")],
            |cpu| cpu.to_string(),
        );
        assert_eq!(linux_core_types(&sys.0), Some((8, 12)));

        // Core i7-13700: 8 performance cores with 2 threads each, then 8 efficiency cores
        let sys = fake_sys(
            "raptor-lake",
            &[("cpu_core", "0-15"), ("cpu_atom", "16-23")],
            |cpu| match cpu {
                0..=15 => format!("{}-{}", cpu / 2 * 2, cpu / 2 * 2 + 1),
                _ => cpu.to_string(),
            },
        );
        assert_eq!(linux_core_types(&sys.0), Some((8, 8)));

        // Core Ultra 9 285H: 6 performance, 8 efficiency and 2 low-power efficiency cores
        let sys = fake_sys(
            "arrow-lake-h",
            &[
                ("cpu_core", "0-5"),
                ("cpu_atom", "6-13"),
                ("cpu_lowpower", "14-15"),
            ],
            |cpu| cpu.to_string(),
        );
        assert_eq!(linux_core_types(&sys.0), Some((6, 10)));

        // chips with one kind of core don't have these files
        assert_eq!(linux_core_types(Path::new("/nonexistent")), None);
    }

    /// A stand-in for Linux's `/sys` folder, deleted when dropped, even if a test fails.
    struct FakeSys(std::path::PathBuf);

    impl Drop for FakeSys {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Builds a fake `/sys` folder where `kinds` pairs each core type with its CPU list, and
    /// `siblings` gives each CPU's siblings list.
    fn fake_sys(name: &str, kinds: &[(&str, &str)], siblings: impl Fn(usize) -> String) -> FakeSys {
        let sys =
            FakeSys(env::temp_dir().join(format!("prime-race-sys-{}-{}", name, process::id())));
        for (kind, cpus) in kinds {
            let pmu = sys.0.join(format!("devices/{}", kind));
            std::fs::create_dir_all(&pmu).unwrap();
            std::fs::write(pmu.join("cpus"), format!("{}\n", cpus)).unwrap();
            for cpu in parse_cpu_list(cpus).unwrap() {
                let topology = sys
                    .0
                    .join(format!("devices/system/cpu/cpu{}/topology", cpu));
                std::fs::create_dir_all(&topology).unwrap();
                std::fs::write(
                    topology.join("thread_siblings_list"),
                    format!("{}\n", siblings(cpu)),
                )
                .unwrap();
            }
        }
        sys
    }

    #[test]
    fn group_adds_thousands_separators() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(1_234_567), "1,234,567");
    }

    #[test]
    fn human_time_picks_a_unit_and_three_digits() {
        assert_eq!(human_time(2.5), "2.50 s");
        assert_eq!(human_time(0.0123), "12.3 ms");
        assert_eq!(human_time(0.000442), "442 µs");
        assert_eq!(human_time(5e-9), "5.00 ns");
    }

    #[test]
    fn primes_are_listed_ten_to_a_line() {
        let ui = Ui {
            out: Paint(false),
            err: Paint(false),
            live: false,
        };
        let mut out = Vec::new();
        ui.write_primes(&mut out, 37, &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31])
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "  11 primes up to 37\n   2   3   5   7  11  13  17  19  23  29\n  31\n\n"
        );
    }
}
