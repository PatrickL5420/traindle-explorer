//! Saving solver output: game-format solutions plus a summary that carries the
//! config, so any run can be reopened, verified and compared later.

use crate::config::SolverConfig;
use crate::driver::{RoundStat, RunResult};
use serde::Serialize;
use std::path::Path;
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

/// Per-puzzle numbers for the web UI in one compact binary file, so a million-puzzle
/// heatmap loads in well under a second (the solution texts are fetched on demand).
///
/// Layout (little-endian): `b"TXG1"`, u32 puzzle count n, u32 flags (bit 0 = cost and
/// count recorded), u32 reserved, then u8 tier[n] (0 = unsolved), u8 length[n]
/// (solution length in characters, capped at 255), u16 cost[n], u16 count[n].
pub fn grid_bytes(digits: usize, rows: impl Iterator<Item = (usize, u8, usize, Option<u32>, Option<u16>)>) -> Vec<u8> {
    let n = 10usize.pow(digits as u32);
    let (mut tier, mut len, mut cost, mut count) = (vec![0u8; n], vec![0u8; n], vec![0u16; n], vec![0u16; n]);
    let mut recorded = true;
    for (i, t, l, c, k) in rows {
        if i >= n {
            continue;
        }
        tier[i] = t;
        len[i] = l.min(255) as u8;
        recorded &= c.is_some() && k.is_some();
        cost[i] = c.unwrap_or(0).min(65535) as u16;
        count[i] = k.unwrap_or(0);
    }
    let mut out = Vec::with_capacity(16 + 6 * n);
    out.extend_from_slice(b"TXG1");
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(recorded as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&tier);
    out.extend_from_slice(&len);
    cost.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    count.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out
}


/// Save a finished run into `dir` (layout: see `store.rs`).
pub fn save_run(dir: &Path, summary: &Summary, numbers: &[Vec<u8>], res: &RunResult) -> std::io::Result<()> {
    let puzzles: Vec<crate::store::Stored> = numbers
        .iter()
        .zip(&res.solutions)
        .filter_map(|(n, s)| {
            let s = s.as_ref()?;
            Some(crate::store::Stored {
                index: std::str::from_utf8(n).ok()?.parse().ok()?,
                solution: &s.expr,
                tier: s.tier,
                cost: Some(s.cost),
                count: Some(s.count),
                alts: s.alts.iter().map(|(c, e)| (Some(*c), e.as_str())).collect(),
            })
        })
        .collect();
    crate::store::write_run(dir, summary, summary.digits, &puzzles)
}
