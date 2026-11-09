//! Saving solver output: game-format solutions plus a summary that carries the
//! config, so any run can be reopened, verified and compared later.

use crate::config::SolverConfig;
use crate::driver::{RoundStat, RunResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct Entry {
    pub number: String,
    pub solution: String,
    pub tier: u8,
    pub cost: u32,
    /// rarity: distinct verified solutions within `alt_slack` of the best
    pub count: u16,
}

/// Alternatives live next to the solutions file (`solutions.json` ->
/// `solutions.alts.jsonl`): too many to load up front for big runs, so the web UI fetches
/// one puzzle's line at a time. One line per puzzle that has alternatives, in puzzle
/// order: `["0123",[[cost,"expr"],...]]`.
pub fn alts_path(solutions: &Path) -> PathBuf {
    let stem = solutions.file_stem().and_then(|s| s.to_str()).unwrap_or("solutions");
    solutions.with_file_name(format!("{stem}.alts.jsonl"))
}

pub fn write_alts(path: &Path, numbers: &[Vec<u8>], res: &RunResult) -> std::io::Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    for (n, s) in numbers.iter().zip(&res.solutions) {
        let Some(s) = s else { continue };
        if s.alts.is_empty() {
            continue;
        }
        let num = std::str::from_utf8(n).unwrap();
        writeln!(w, "{}", serde_json::to_string(&(num, &s.alts))?)?;
    }
    w.flush()
}

#[derive(Serialize)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub created: String,
    pub digits: usize,
    pub target: String,
    pub tier_names: Vec<String>,
    pub total: usize,
    pub solved: usize,
    /// solutions per tier, index 0 = tier 1
    pub tier_counts: Vec<usize>,
    pub unsolved_count: usize,
    /// first 2000 unsolved puzzles
    pub unsolved: Vec<String>,
    pub elapsed_s: f64,
    pub cancelled: bool,
    pub threads: usize,
    pub rounds: Vec<RoundStat>,
    pub config: SolverConfig,
    /// set when this run is one target of a sweep
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep: Option<SweepInfo>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SweepInfo {
    pub id: String,
    pub name: String,
    /// 0-based position of this run's target in the sweep
    pub index: usize,
    pub count: usize,
}

/// UTC timestamp as (ISO-8601 string, compact id prefix).
pub fn now_stamp() -> (String, String) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let (hh, mm, ss) = (rem / 3600, rem / 60 % 60, rem % 60);
    (format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"), format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}"))
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    s.chars().take(40).collect()
}

/// Run ids are used as directory names: only allow what `new_run_id` produces.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn new_run_id(runs_dir: &Path, name: &str) -> String {
    let (_, compact) = now_stamp();
    let base = if slug(name).is_empty() { compact } else { format!("{compact}-{}", slug(name)) };
    let mut id = base.clone();
    let mut k = 2;
    while runs_dir.join(&id).exists() {
        id = format!("{base}-{k}");
        k += 1;
    }
    id
}

pub fn write_json<T: Serialize>(path: &Path, v: &T, pretty: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let s = if pretty { serde_json::to_string_pretty(v)? } else { serde_json::to_string(v)? };
    std::fs::write(path, s)
}

pub fn build_outputs(
    id: &str,
    numbers: &[Vec<u8>],
    sc: &SolverConfig,
    res: &RunResult,
    elapsed_s: f64,
    sweep: Option<SweepInfo>,
) -> (Vec<Entry>, Summary) {
    let mut entries = Vec::new();
    let mut unsolved = Vec::new();
    let mut tier_counts = vec![0usize; sc.tiers.len()];
    for (n, s) in numbers.iter().zip(&res.solutions) {
        let num = String::from_utf8(n.clone()).unwrap();
        match s {
            Some(s) => {
                if (1..=tier_counts.len()).contains(&(s.tier as usize)) {
                    tier_counts[s.tier as usize - 1] += 1;
                }
                entries.push(Entry { number: num, solution: s.expr.clone(), tier: s.tier, cost: s.cost, count: s.count });
            }
            None => unsolved.push(num),
        }
    }
    let summary = Summary {
        id: id.to_string(),
        name: sc.name.clone(),
        created: now_stamp().0,
        digits: sc.digits,
        target: sc.target.clone(),
        tier_names: sc.tiers.iter().map(|t| t.name.clone()).collect(),
        total: numbers.len(),
        solved: entries.len(),
        tier_counts,
        unsolved_count: unsolved.len(),
        unsolved: unsolved.into_iter().take(2000).collect(),
        elapsed_s,
        cancelled: res.cancelled,
        threads: rayon::current_num_threads(),
        rounds: res.rounds.clone(),
        config: sc.clone(),
        sweep,
    };
    (entries, summary)
}

/// Write `config.json`, `solutions.json` (compact: it can hold a million puzzles),
/// `solutions.alts.jsonl` and `summary.json` into `dir`.
pub fn save_run(dir: &PathBuf, entries: &[Entry], summary: &Summary, numbers: &[Vec<u8>], res: &RunResult) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_json(&dir.join("config.json"), &summary.config, true)?;
    let sols = dir.join("solutions.json");
    write_json(&sols, &entries, false)?;
    write_alts(&alts_path(&sols), numbers, res)?;
    write_json(&dir.join("summary.json"), summary, true)
}
