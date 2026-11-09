//! Cross-number memoisation: one value set per digit substring.
//!
//! A substring's set depends only on its digits, so all puzzles share them: for
//! n-digit puzzles at most 10 + 100 + ... + 10^(n-1) sets (lengths 1..n-1; a 1-digit
//! puzzle uses its own length-1 set). Levels are built in parallel with a barrier
//! between them (longer substrings read the frozen shorter ones).

use crate::progress::Progress;
use crate::span::{build_set, Ctx, SpanSet};
use rayon::prelude::*;
use std::sync::atomic::Ordering;

/// Longest substring with a set (6-digit puzzles split into at most 5 + 1).
pub const MAX_SUB: usize = 5;
const OFF: [usize; MAX_SUB + 2] = [0, 0, 10, 110, 1110, 11110, 111110];
pub const N_CODES: usize = OFF[MAX_SUB + 1];

#[inline]
pub fn code(d: &[u8]) -> usize {
    let mut v = 0usize;
    for &c in d {
        v = v * 10 + (c - b'0') as usize;
    }
    OFF[d.len()] + v
}

pub fn digits_of(code: usize) -> Vec<u8> {
    let len = (1..=MAX_SUB).find(|&l| code < OFF[l + 1]).expect("code out of range");
    format!("{:0width$}", code - OFF[len], width = len).into_bytes()
}

/// Mark every substring a round needs for these puzzles.
pub fn needed_for(numbers: &[Vec<u8>]) -> Vec<bool> {
    let mut needed = vec![false; N_CODES];
    for n in numbers {
        let maxlen = (n.len().max(2) - 1).min(MAX_SUB);
        for len in 1..=maxlen {
            for start in 0..=n.len() - len {
                needed[code(&n[start..start + len])] = true;
            }
        }
    }
    needed
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CacheStats {
    pub sets: usize,
    pub entries: usize,
    pub max_entries: usize,
    pub truncated: usize,
    pub candidates: u64,
    pub mem_bytes: usize,
}

pub struct Cache {
    sets: Vec<Option<SpanSet>>,
}

impl Cache {
    #[inline]
    pub fn get(&self, d: &[u8]) -> &SpanSet {
        self.sets[code(d)].as_ref().expect("substring set not built")
    }

    pub fn build(needed: &[bool], ctx: &Ctx, progress: &Progress) -> Cache {
        let mut cache = Cache { sets: (0..N_CODES).map(|_| None).collect() };
        let total = needed.iter().filter(|&&b| b).count();
        progress.stage("building value sets", total);
        for len in 1..=MAX_SUB {
            let codes: Vec<usize> = (OFF[len]..OFF[len + 1]).filter(|&c| needed[c]).collect();
            if codes.is_empty() {
                continue;
            }
            let built: Vec<(usize, SpanSet)> = {
                let cref = &cache;
                let get = |s: &[u8]| cref.get(s);
                codes
                    .par_iter()
                    .with_max_len(1)
                    .map(|&c| {
                        // cancelled: build an empty set so the round winds down quickly
                        let set = if progress.cancelled() {
                            SpanSet::empty()
                        } else {
                            build_set(&digits_of(c), &get, ctx)
                        };
                        progress.done.fetch_add(1, Ordering::Relaxed);
                        (c, set)
                    })
                    .collect()
            };
            for (c, s) in built {
                cache.sets[c] = Some(s);
            }
        }
        cache
    }

    pub fn stats(&self) -> CacheStats {
        let mut st = CacheStats::default();
        for s in self.sets.iter().flatten() {
            st.sets += 1;
            st.entries += s.len();
            st.max_entries = st.max_entries.max(s.len());
            st.truncated += s.truncated_at.is_some() as usize;
            st.candidates += s.candidates;
            st.mem_bytes += s.mem_bytes();
        }
        st
    }
}
