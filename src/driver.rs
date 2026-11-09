//! Round orchestration. Flat rounds (before brackets) enumerate directly; later rounds
//! build the shared substring cache for still-unsolved puzzles and run the root join.

use crate::cache::{needed_for, Cache, CacheStats};
use crate::config::{rounds, Config, SolverConfig};
use crate::collect::Found;
use crate::flat::solve_flat;
use crate::ops::MOD;
use crate::pretarget::pretargets;
use crate::progress::Progress;
use crate::root::{init_sieve, root_join, RootPrep};
use crate::span::Ctx;
use rayon::prelude::*;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Solved {
    pub expr: String,
    pub tier: u8,
    pub cost: u32,
    /// distinct verified solutions within the rarity window (incl. this one)
    pub count: u16,
    pub alts: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct RoundStat {
    pub label: String,
    pub numbers_in: usize,
    pub solved: usize,
    pub build_ms: f64,
    pub join_ms: f64,
    pub sets: usize,
    pub entries: usize,
    pub max_set: usize,
    pub truncated_sets: usize,
    pub candidates: u64,
    pub mem_mb: f64,
    pub pretargets: usize,
}

pub struct RunResult {
    pub solutions: Vec<Option<Solved>>,
    pub rounds: Vec<RoundStat>,
    pub cancelled: bool,
}

fn solved(f: Found, sc: &SolverConfig) -> Solved {
    Solved {
        expr: f.expr.render(),
        tier: sc.tier_of_mask(f.expr.features()),
        cost: f.score >> 12,
        count: f.count,
        alts: f.alts,
    }
}

/// All puzzles with `digits` digits, as ASCII digit strings.
pub fn all_numbers(digits: usize) -> Vec<Vec<u8>> {
    (0..10usize.pow(digits as u32)).map(|i| format!("{i:0digits$}").into_bytes()).collect()
}

pub fn run(numbers: &[Vec<u8>], sc: &SolverConfig, progress: &Progress, verbose: bool) -> RunResult {
    let cfg = Config::from(sc);
    let rounds = rounds(sc);
    let mut out: Vec<Option<Solved>> = vec![None; numbers.len()];
    let mut stats = Vec::new();
    progress.total.store(numbers.len(), Ordering::Relaxed);
    progress.solved.store(0, Ordering::Relaxed);
    progress.round_count.store(rounds.len(), Ordering::Relaxed);

    for (ri, round) in rounds.iter().enumerate() {
        if progress.cancelled() {
            break;
        }
        let unsolved: Vec<usize> = (0..numbers.len()).filter(|&i| out[i].is_none()).collect();
        if unsolved.is_empty() {
            break;
        }
        progress.round(ri + 1, &round.label);
        let mut st = RoundStat {
            label: round.label.clone(),
            numbers_in: unsolved.len(),
            solved: 0,
            build_ms: 0.0,
            join_ms: 0.0,
            sets: 0,
            entries: 0,
            max_set: 0,
            truncated_sets: 0,
            candidates: 0,
            mem_mb: 0.0,
            pretargets: 0,
        };
        let found: Vec<(usize, Option<Solved>)> = if round.flat {
            let tj = Instant::now();
            progress.stage("enumerating flat expressions", unsolved.len());
            let f = unsolved
                .par_iter()
                .map(|&i| {
                    let r = if progress.cancelled() {
                        None
                    } else {
                        solve_flat(&numbers[i], round, &cfg).map(|f| solved(f, sc))
                    };
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    if r.is_some() {
                        progress.solved.fetch_add(1, Ordering::Relaxed);
                    }
                    (i, r)
                })
                .collect();
            st.join_ms = tj.elapsed().as_secs_f64() * 1e3;
            f
        } else {
            let ctx = Ctx::new(&cfg, round);
            if ctx.binops.contains(&MOD) {
                init_sieve();
            }
            let un_nums: Vec<Vec<u8>> = unsolved.iter().map(|&i| numbers[i].clone()).collect();
            let tb = Instant::now();
            let cache = Cache::build(&needed_for(&un_nums), &ctx, progress);
            st.build_ms = tb.elapsed().as_secs_f64() * 1e3;
            let prep = RootPrep::new(pretargets(round, &cfg));
            let tj = Instant::now();
            progress.stage("joining", unsolved.len());
            let parallel_inner = unsolved.len() < 4 * rayon::current_num_threads();
            let f = unsolved
                .par_iter()
                .with_max_len(1)
                .map(|&i| {
                    let n = &numbers[i];
                    let r = if progress.cancelled() {
                        None
                    } else {
                        root_join(n, &cache, &prep, &ctx, parallel_inner).map(|f| solved(f, sc))
                    };
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    if r.is_some() {
                        progress.solved.fetch_add(1, Ordering::Relaxed);
                    }
                    (i, r)
                })
                .collect();
            st.join_ms = tj.elapsed().as_secs_f64() * 1e3;
            let cs: CacheStats = cache.stats();
            st.sets = cs.sets;
            st.entries = cs.entries;
            st.max_set = cs.max_entries;
            st.truncated_sets = cs.truncated;
            st.candidates = cs.candidates;
            st.mem_mb = cs.mem_bytes as f64 / 1e6;
            st.pretargets = prep.pts.len();
            f
        };
        for (i, s) in found {
            if s.is_some() {
                st.solved += 1;
                out[i] = s;
            }
        }
        let left = unsolved.len() - st.solved;
        let line = format!(
            "{:<22} solved {:>6}  left {:>6}  ({:.2} s)",
            st.label,
            st.solved,
            left,
            (st.build_ms + st.join_ms) / 1e3
        );
        if verbose {
            eprintln!(
                "{line} | build {:.1} ms join {:.1} ms | sets {} entries {} max {} trunc {} cand {} mem {:.1} MB pts {}",
                st.build_ms, st.join_ms, st.sets, st.entries, st.max_set, st.truncated_sets, st.candidates, st.mem_mb, st.pretargets
            );
        } else {
            eprintln!("{line}");
        }
        progress.log(line);
        stats.push(st);
    }
    RunResult { solutions: out, rounds: stats, cancelled: progress.cancelled() }
}
