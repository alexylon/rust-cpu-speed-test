//! Terminal output: a header describing the machine and the run, a live
//! progress bar while each variant runs, then a results table and a score.
//!
//! Results go to stdout and the progress bar to stderr. Colours and the
//! progress bar are only used on a terminal, and `NO_COLOR` turns colours off.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crate::RunResult;

/// Width of the first column, holding variant names in the table and progress bar.
const LABEL_WIDTH: usize = 21;
const BAR_WIDTH: usize = 30;
/// Covers the longest progress line, so that it can be erased.
const PROGRESS_WIDTH: usize = 2 + LABEL_WIDTH + BAR_WIDTH + 18;

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
            |name: &str, value: String| println!("  {}  {}", p.dim(&format!("{:<8}", name)), value);

        println!();
        println!(
            "  {}  {}",
            p.bold("CPU Speed Test"),
            p.dim("prime sieve benchmark")
        );
        println!();
        field(
            "CPU",
            cpu_model().unwrap_or_else(|| env::consts::ARCH.to_string()),
        );
        field(
            "Cores",
            format!(
                "{} logical · {} physical",
                num_cpus::get(),
                num_cpus::get_physical()
            ),
        );
        field(
            "Threads",
            if auto_threads {
                format!("{} {}", threads, p.dim("(all logical cores)"))
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
        println!();
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
        println!("  {}", self.out.dim(&headings));
    }

    /// A finished run, as a row of the results table.
    pub fn row(&self, run: &RunResult) {
        let p = self.out;
        let check = match run.valid {
            Some(true) => p.green("✓"),
            Some(false) => p.red("✗"),
            None => p.yellow("?"),
        };
        println!(
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
        println!();
        println!(
            "  {}  {}  {}",
            p.bold("Score"),
            p.green(&p.bold(&format!("{} passes/s", group(best.rate().round() as usize)))),
            p.dim(&format!("({})", detail))
        );
        if runs.iter().any(|run| run.valid == Some(false)) {
            println!(
                "  {}",
                p.red("✗ some runs counted the wrong number of primes")
            );
        }
        if runs.iter().any(|run| run.valid.is_none()) {
            println!(
                "  {}",
                p.yellow("? prime counts are only checked when the limit is a power of 10")
            );
        }
        println!();
    }

    /// Every prime found, in right-aligned columns.
    pub fn primes(&self, limit: usize, primes: &[usize]) {
        // stop quietly if stdout closes early, e.g. when piped into `head`
        let _ = self.write_primes(&mut io::BufWriter::new(io::stdout().lock()), limit, primes);
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
