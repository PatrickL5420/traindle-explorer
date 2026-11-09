//! Verified solutions for one puzzle: the best one plus the alternatives inside the
//! rarity window (cost <= best cost + slack).
//!
//! Callers verify a candidate (f64 + 256-bit exactness) before `add`; only verified
//! solutions ever enter the list, so the window is anchored to a genuine best.

use crate::expr::Expr;
use hashbrown::HashSet;

/// Most solutions kept per puzzle; the rarity count saturates here.
pub const CAP: usize = 100;
/// Alternatives stored with a run (besides the best).
pub const ALTS: usize = 4;

pub struct Found {
    pub expr: Expr,
    pub score: u32,
    /// next-best distinct solutions, best first
    pub alts: Vec<String>,
    /// distinct verified solutions within the window (incl. the best), capped at CAP
    pub count: u16,
}

pub struct Collector {
    pub slack: u32,
    /// sorted by (score, text)
    list: Vec<(u32, String, Expr)>,
    /// every text already offered, verified or not
    seen: HashSet<String>,
}

impl Collector {
    pub fn new(slack: u32) -> Self {
        Collector { slack, list: Vec::new(), seen: HashSet::new() }
    }

    /// Highest cost still inside the window around `best` (a score).
    #[inline]
    pub fn cost_limit(&self, best: u32) -> u32 {
        (best >> 12).saturating_add(self.slack)
    }

    /// Cheap pre-check before rendering: could a candidate of this score still count?
    #[inline]
    pub fn in_window(&self, score: u32, best: u32) -> bool {
        if self.slack == 0 {
            score <= best
        } else {
            score >> 12 <= self.cost_limit(best)
        }
    }

    /// Should this rendered candidate be verified? Marks the text as seen.
    pub fn wants(&mut self, score: u32, text: &str, best: u32) -> bool {
        if !self.in_window(score, best) {
            return false;
        }
        if self.slack == 0 {
            // only the best matters: skip anything not strictly better than what we hold
            if let Some((s, t, _)) = self.list.first() {
                if (score, text) >= (*s, t.as_str()) {
                    return false;
                }
            }
            return true;
        }
        if self.seen.contains(text) {
            return false;
        }
        self.seen.insert(text.to_string());
        true
    }

    /// Add a verified solution; `best` is the best verified score known anywhere.
    pub fn add(&mut self, score: u32, text: String, e: Expr, best: u32) {
        let best = best.min(score);
        let key = (score, text.as_str());
        let pos = self.list.partition_point(|(s, t, _)| (*s, t.as_str()) < key);
        if self.list.get(pos).is_some_and(|(s, t, _)| *s == score && *t == text) {
            return;
        }
        self.list.insert(pos, (score, text, e));
        self.prune(best);
    }

    fn prune(&mut self, best: u32) {
        if self.slack == 0 {
            self.list.truncate(1);
            return;
        }
        let lim = self.cost_limit(best);
        self.list.retain(|(s, _, _)| s >> 12 <= lim);
        self.list.truncate(CAP);
    }

    pub fn best_score(&self) -> u32 {
        self.list.first().map_or(u32::MAX, |x| x.0)
    }

    pub fn merge(&mut self, other: Collector) {
        let best = self.best_score().min(other.best_score());
        for (s, t, e) in other.list {
            self.add(s, t, e, best);
        }
        self.prune(best);
    }

    pub fn finish(mut self) -> Option<Found> {
        let best = self.best_score();
        self.prune(best);
        let count = self.list.len() as u16;
        let mut it = self.list.into_iter();
        let (score, _, expr) = it.next()?;
        let alts = it.take(ALTS).map(|x| x.1).collect();
        Some(Found { expr, score, alts, count })
    }
}
