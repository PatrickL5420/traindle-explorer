//! Traindle Explorer: for every n-digit string, find the simplest expression that uses
//! the digits in order and equals the target exactly, trying operator tiers in order.
//!
//!   traindle-explorer               solve the default game (4 digits, target 10)
//!   traindle-explorer serve         open the web UI

mod cache;
mod collect;
mod config;
mod driver;
mod expr;
mod flat;
mod fuzz;
mod key;
mod ops;
mod pretarget;
mod progress;
mod root;
mod runs;
mod server;
mod span;

use clap::{Args as ClapArgs, Parser, Subcommand};
use config::{Profile, SolverConfig};
use std::path::PathBuf;
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(version, about = "Traindle Explorer: make a target from n digits, in order")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Worker threads (default: all logical CPUs)
    #[arg(long, global = true)]
    threads: Option<usize>,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Start the local web UI
    Serve {
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Where runs are saved
        #[arg(long, default_value = "runs")]
        runs_dir: PathBuf,
        /// Don't open a browser window
        #[arg(long)]
        no_open: bool,
    },
}

#[derive(ClapArgs, Debug)]
struct SolveArgs {
    /// Output solutions JSON
    #[arg(long, default_value = "output/solutions.json")]
    out: PathBuf,
    /// Output summary JSON (includes the config, for verify/verify.mjs)
    #[arg(long, default_value = "output/summary.json")]
    summary: PathBuf,
    /// Solver config JSON (as saved in runs/<id>/config.json); flags below override it
    #[arg(long)]
    config: Option<PathBuf>,
    /// Digits per puzzle (1-6)
    #[arg(long)]
    digits: Option<usize>,
    /// Target value (e.g. 10, 24, -3, 0.5)
    #[arg(long, allow_hyphen_values = true)]
    target: Option<String>,
    /// Only use the first N tiers
    #[arg(long)]
    max_tier: Option<usize>,
    /// Only solve these numbers, e.g. --only 0000,0677,7677
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,
    /// Search budget profile
    #[arg(long, value_enum)]
    profile: Option<Profile>,
    /// Largest intermediate magnitude kept
    #[arg(long)]
    max_abs: Option<f64>,
    /// Cost per joined digit in a concatenated literal
    #[arg(long)]
    concat_cost: Option<u32>,
    /// Allow factorial of non-integers (gamma function)
    #[arg(long)]
    gamma: bool,
    /// Trig search depth: number of trig rounds, 1-3 (default 2; 3 adds a slow last resort)
    #[arg(long)]
    trig_rounds: Option<u8>,
    /// Rarity window: count verified solutions costing at most best + N (0 = off)
    #[arg(long)]
    alt_slack: Option<u32>,
    /// Print detailed per-round statistics
    #[arg(long)]
    stats: bool,
    /// Write N random expression tests (for verify/exprcheck.mjs) and exit
    #[arg(long)]
    emit_expr_tests: Option<usize>,
    /// Seed for --emit-expr-tests
    #[arg(long, default_value_t = 0x5eed_1234_abcd_ef01)]
    seed: u64,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    if let Some(n) = cli.threads {
        rayon::ThreadPoolBuilder::new().num_threads(n).build_global().expect("thread pool");
    }
    match cli.cmd {
        Some(Cmd::Serve { port, runs_dir, no_open }) => server::serve(port, runs_dir, !no_open),
        None => solve(cli.solve),
    }
}

fn solve(args: SolveArgs) -> std::io::Result<()> {
    if let Some(n) = args.emit_expr_tests {
        let tests = fuzz::expr_tests(n, args.seed);
        let path = PathBuf::from("output/expr_tests.json");
        runs::write_json(&path, &tests, false)?;
        eprintln!("wrote {} expression tests to {}", n, path.display());
        return Ok(());
    }

    let mut sc: SolverConfig = match &args.config {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?).map_err(std::io::Error::other)?,
        None => SolverConfig::default(),
    };
    if let Some(d) = args.digits {
        sc.digits = d;
    }
    if let Some(t) = &args.target {
        sc.target = t.clone();
    }
    if let Some(m) = args.max_tier {
        sc.tiers.truncate(m.max(1));
    }
    if let Some(p) = args.profile {
        sc.profile = p;
    }
    if let Some(m) = args.max_abs {
        sc.max_abs = m;
    }
    if let Some(c) = args.concat_cost {
        sc.weights.insert("concat".into(), c);
    }
    sc.gamma |= args.gamma;
    if let Some(t) = args.trig_rounds {
        sc.trig_rounds = t;
    }
    if let Some(a) = args.alt_slack {
        sc.alt_slack = a;
    }
    sc.normalize().map_err(std::io::Error::other)?;

    let numbers: Vec<Vec<u8>> = if args.only.is_empty() {
        driver::all_numbers(sc.digits)
    } else {
        args.only
            .iter()
            .map(|s| {
                let s = s.trim();
                assert!(s.len() == sc.digits && s.bytes().all(|c| c.is_ascii_digit()), "bad number {s:?}");
                s.as_bytes().to_vec()
            })
            .collect()
    };

    let threads = rayon::current_num_threads();
    eprintln!(
        "solving {} puzzles ({} digits, target {}) on {} threads",
        numbers.len(),
        sc.digits,
        sc.target,
        threads
    );
    let t0 = Instant::now();
    let progress = progress::Progress::default();
    let res = driver::run(&numbers, &sc, &progress, args.stats);
    let elapsed = t0.elapsed().as_secs_f64();
    let (entries, summary) = runs::build_outputs("cli", &numbers, &sc, &res, elapsed, None);

    if !args.only.is_empty() {
        let by: std::collections::HashMap<&str, &runs::Entry> = entries.iter().map(|e| (e.number.as_str(), e)).collect();
        for n in &numbers {
            let n = std::str::from_utf8(n).unwrap();
            match by.get(n) {
                Some(e) => println!("{n}  T{:<2} {}", e.tier, e.solution),
                None => println!("{n}  unsolved"),
            }
        }
    }
    runs::write_json(&args.out, &entries, false)?;
    runs::write_alts(&runs::alts_path(&args.out), &numbers, &res)?;
    runs::write_json(&args.summary, &summary, true)?;

    eprintln!("\nsolved {}/{} in {:.2} s", summary.solved, summary.total, elapsed);
    for (i, (name, n)) in summary.tier_names.iter().zip(&summary.tier_counts).enumerate() {
        eprintln!("  tier {:>2} {name:<28} {n:>6}", i + 1);
    }
    if summary.unsolved_count > 0 {
        let shown: Vec<&str> = summary.unsolved.iter().take(40).map(|s| s.as_str()).collect();
        eprintln!("unsolved ({}): {}{}", summary.unsolved_count, shown.join(" "), if summary.unsolved_count > 40 { " ..." } else { "" });
    }
    eprintln!("wrote {} and {}", args.out.display(), args.summary.display());
    Ok(())
}
