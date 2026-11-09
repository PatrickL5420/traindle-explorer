//! Root join: for one 4-digit number, find the cheapest `chain(a op b) = 10` where `a`
//! and `b` come from the value sets of a split (1|3, 2|2, 3|1) and `chain` is a
//! pretarget's unary chain.
//!
//! Invertible ops (+ - * / ^) iterate the smaller side and hash-probe the other for
//! the required partner value (meet in the middle) instead of enumerating L x R.
//! `%` uses divisor enumeration, combinations/permutations use (n, k) tables.
//! Branch-and-bound on cost, widened by the rarity window (`Config::slack`).

use crate::cache::Cache;
use crate::collect::{Collector, Found};
use crate::config::Config;
use crate::expr::Expr;
use crate::ops::*;
use crate::pretarget::{apply_chain, Pretarget};
use crate::span::{bin_len, mk_score, parse_digits, Ctx, SpanSet};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

/// Candidate kind for 1-digit puzzles: the target found directly in the digit's set.
const SET_ENTRY: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cand {
    pub score: u32,
    pub split: u8,
    pub op: u8, // LIT = the whole literal, SET_ENTRY = entry `ia` of the whole number's set
    pub ia: u32,
    pub ib: u32,
    pub pt: u16,
}

impl Cand {
    pub fn to_expr(&self, num: &[u8], cache: &Cache, pts: &[Pretarget]) -> Expr {
        let core = if self.op == LIT {
            Expr::lit(num)
        } else if self.op == SET_ENTRY {
            Expr::from_set(cache, num, self.ia)
        } else {
            let s = self.split as usize;
            Expr::bin(self.op, Expr::from_set(cache, &num[..s], self.ia), Expr::from_set(cache, &num[s..], self.ib))
        };
        pts[self.pt as usize].chain.iter().fold(core, |e, &u| Expr::un(u, e))
    }
}

/// Per-round data shared by all root joins.
pub struct RootPrep {
    pub pts: Vec<Pretarget>,
    comb: Vec<Vec<(u64, u64)>>,
    perm: Vec<Vec<(u64, u64)>>,
}

fn nk_pairs(op: u8, t: f64) -> Vec<(u64, u64)> {
    let mut v = Vec::new();
    if !(t >= 1.0 && t == t.round() && t < 1e15) {
        return v;
    }
    let ti = t as u64;
    for n in 0..=120u64 {
        for k in 0..=n {
            if comb_perm(op, n as f64, k as f64) == Some(t) {
                v.push((n, k));
            }
        }
    }
    if ti > 120 {
        v.push((ti, 1));
        if op == COMB {
            v.push((ti, ti - 1));
        }
    }
    v
}

impl RootPrep {
    pub fn new(pts: Vec<Pretarget>) -> Self {
        let comb = pts.iter().map(|p| nk_pairs(COMB, p.t)).collect();
        let perm = pts.iter().map(|p| nk_pairs(PERM, p.t)).collect();
        RootPrep { pts, comb, perm }
    }
}

/// Best *verified* solution for one number.
///
/// A candidate is verified (f64 value + 256-bit exactness) the moment it
/// would beat the current best, and only verified scores tighten the bound shared by
/// all jobs. An unverified bound would be wrong: a cheap candidate that later fails the
/// exact check (a float artefact) would prune every costlier genuine solution.
/// Ties on score are broken by the rendered text, so results are deterministic.
struct Best<'a> {
    budget: u32,
    /// best verified score (cost << 12 | length) across jobs
    shared: &'a AtomicU32,
    found: Collector,
    num: &'a [u8],
    cache: &'a Cache,
    prep: &'a RootPrep,
    cfg: &'a Config,
}

impl Best<'_> {
    /// Highest cost still worth exploring (inclusive): the round budget, or the
    /// rarity window around the best verified solution.
    #[inline]
    fn bound(&self) -> u32 {
        self.budget.min(self.found.cost_limit(self.shared.load(Ordering::Relaxed)))
    }
    fn push(&mut self, c: Cand) {
        let best = self.shared.load(Ordering::Relaxed);
        if c.score >> 12 > self.budget || !self.found.in_window(c.score, best) {
            return;
        }
        let e = c.to_expr(self.num, self.cache, &self.prep.pts);
        let text = e.render();
        if !self.found.wants(c.score, &text, best) {
            return;
        }
        let ok = e.eval().is_some_and(|v| (v - self.cfg.target).abs() <= self.cfg.tol) && e.verify_exact(&self.cfg.target_str);
        if ok {
            let best = self.shared.fetch_min(c.score, Ordering::Relaxed).min(c.score);
            self.found.add(c.score, text, e, best);
        }
    }
}

struct J<'a> {
    l: &'a SpanSet,
    r: &'a SpanSet,
    split: u8,
    op: u8,
    w: u32,
    pi: u16,
    p: &'a Pretarget,
    lim: &'a Limits,
    tol: f64,
    target: f64,
}

#[inline]
fn try_pair(j: &J, ia: usize, ib: usize, top: &mut Best) {
    let (l, r) = (j.l, j.r);
    let cost = l.cost(ia) + r.cost(ib) + j.w + j.p.cost;
    if cost > top.bound() {
        return;
    }
    let Some((v, _)) = bin(j.op, l.val[ia], l.flags[ia], r.val[ib], r.flags[ib], j.lim) else { return };
    let Some(fv) = apply_chain(v, &j.p.chain) else { return };
    if (fv - j.target).abs() > j.tol {
        return;
    }
    let len = bin_len(j.op, l.rlen(ia), l.node[ia].op, r.rlen(ib), r.node[ib].op) + j.p.len;
    top.push(Cand { score: mk_score(cost, len), split: j.split, op: j.op, ia: ia as u32, ib: ib as u32, pt: j.pi });
}

#[inline]
fn probe_right(j: &J, ia: usize, y: f64, top: &mut Best) {
    let mut buf = [0u32; 3];
    let n = j.r.find(y, &mut buf);
    for &ib in &buf[..n] {
        try_pair(j, ia, ib as usize, top);
    }
}

#[inline]
fn probe_left(j: &J, ib: usize, x: f64, top: &mut Best) {
    let mut buf = [0u32; 3];
    let n = j.l.find(x, &mut buf);
    for &ia in &buf[..n] {
        try_pair(j, ia as usize, ib, top);
    }
}

/// Most entries one join job scans on its iterated side (cheapest first). Only bites
/// when both sides of a split are huge, e.g. 1|1 splits of short puzzles.
const PROBE_CAP: usize = 2_000_000;

/// (entries of L within budget, entries of R within budget), or None if hopeless.
#[inline]
fn budgets(j: &J, top: &Best) -> Option<(usize, usize)> {
    let base = j.w + j.p.cost;
    let bound = top.bound();
    let (lmin, rmin) = (j.l.min_cost(), j.r.min_cost());
    if lmin + rmin + base > bound {
        return None;
    }
    let (nl, nr) = (j.l.count_le(bound - base - rmin), j.r.count_le(bound - base - lmin));
    // the iterated side is the smaller one; cap it, keep the probed side whole
    if nl <= nr {
        Some((nl.min(PROBE_CAP), nr))
    } else {
        Some((nl, nr.min(PROBE_CAP)))
    }
}

fn join_invertible(j: &J, top: &mut Best) {
    let Some((nl, nr)) = budgets(j, top) else { return };
    let t = j.p.t;
    let base = j.w + j.p.cost;
    let (l, r) = (j.l, j.r);
    if nl <= nr {
        let rmin = r.min_cost();
        for ia in 0..nl {
            if l.cost(ia) + rmin + base > top.bound() {
                break;
            }
            let a = l.val[ia];
            match j.op {
                ADD => probe_right(j, ia, t - a, top),
                SUB => probe_right(j, ia, a - t, top),
                MUL => {
                    if a != 0.0 {
                        probe_right(j, ia, t / a, top)
                    }
                }
                DIV => {
                    if t != 0.0 {
                        probe_right(j, ia, a / t, top)
                    }
                }
                POW => {
                    if a > 0.0 && a != 1.0 && t > 0.0 {
                        probe_right(j, ia, t.ln() / a.ln(), top);
                    } else if a < 0.0 && a != -1.0 && t != 0.0 {
                        let b = (t.abs().ln() / (-a).ln()).round();
                        if b.abs() <= 64.0 {
                            probe_right(j, ia, b, top);
                        }
                    }
                }
                _ => {}
            }
        }
    } else {
        let lmin = l.min_cost();
        for ib in 0..nr {
            if r.cost(ib) + lmin + base > top.bound() {
                break;
            }
            let b = r.val[ib];
            match j.op {
                ADD => probe_left(j, ib, t - b, top),
                SUB => probe_left(j, ib, t + b, top),
                MUL => {
                    if b != 0.0 {
                        probe_left(j, ib, t / b, top)
                    }
                }
                DIV => probe_left(j, ib, t * b, top),
                POW => {
                    if b != 0.0 {
                        let even = b == b.round() && (b as i64) % 2 == 0;
                        let odd = b == b.round() && (b as i64) % 2 != 0;
                        if t > 0.0 {
                            let a = libm::pow(t, 1.0 / b);
                            probe_left(j, ib, a, top);
                            if even {
                                probe_left(j, ib, -a, top);
                            }
                        } else if t < 0.0 && odd {
                            probe_left(j, ib, -libm::pow(-t, 1.0 / b), top);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

const SIEVE_N: usize = 11_000_000;
static SPF: OnceLock<Vec<u32>> = OnceLock::new();

/// Smallest-prime-factor sieve, built on first use (only when `%` is in play).
fn spf() -> &'static [u32] {
    SPF.get_or_init(|| {
        let mut s = vec![0u32; SIEVE_N];
        for i in 2..SIEVE_N {
            if s[i] == 0 {
                let mut k = i;
                while k < SIEVE_N {
                    if s[k] == 0 {
                        s[k] = i as u32;
                    }
                    k += i;
                }
            }
        }
        s
    })
}

pub fn init_sieve() {
    let _ = spf();
}

fn divisors(mut n: u64, out: &mut Vec<u64>) {
    out.clear();
    out.push(1);
    let s = spf();
    while n > 1 {
        let p = s[n as usize] as u64;
        let mut e = 0;
        while n % p == 0 {
            n /= p;
            e += 1;
        }
        let len = out.len();
        let mut pk = 1;
        for _ in 0..e {
            pk *= p;
            for i in 0..len {
                out.push(out[i] * pk);
            }
        }
    }
}

fn join_mod(j: &J, top: &mut Best) {
    let Some((nl, nr)) = budgets(j, top) else { return };
    let t = j.p.t;
    let base = j.w + j.p.cost;
    let (l, r) = (j.l, j.r);
    let rmin = r.min_cost();
    if nr <= 64 {
        // small divisor side: plain pair enumeration is cheapest
        for ia in 0..nl {
            if l.cost(ia) + rmin + base > top.bound() {
                break;
            }
            for ib in 0..nr {
                try_pair(j, ia, ib, top);
            }
        }
        return;
    }
    // A: iterate the dividend a; a % b = t  =>  b divides (a - t)
    let mut divs = Vec::new();
    for ia in 0..nl {
        if l.cost(ia) + rmin + base > top.bound() {
            break;
        }
        let a = l.val[ia];
        let d = a - t;
        if d == 0.0 {
            // a % b = a for any b beyond a with the same sign: take the cheapest
            let lim = r.count_le(top.bound().saturating_sub(l.cost(ia) + base));
            let mut found = 0;
            for ib in 0..lim {
                let b = r.val[ib];
                if (a > 0.0 && b > a) || (a < 0.0 && b < a) {
                    try_pair(j, ia, ib, top);
                    found += 1;
                    if found >= 2 {
                        break;
                    }
                }
            }
        } else if d == d.round() && d.abs() < SIEVE_N as f64 {
            divisors(d.abs() as u64, &mut divs);
            for &dv in &divs {
                probe_right(j, ia, dv as f64, top);
                probe_right(j, ia, -(dv as f64), top);
            }
        } else {
            for k in 1..=24 {
                let b = d / k as f64;
                probe_right(j, ia, b, top);
                probe_right(j, ia, -b, top);
            }
        }
    }
    // B: iterate the divisor b with small quotients (non-integer dividends)
    if nr <= 4096 {
        let lmin = l.min_cost();
        for ib in 0..nr {
            if r.cost(ib) + lmin + base > top.bound() {
                break;
            }
            let b = r.val[ib];
            if b == 0.0 {
                continue;
            }
            for k in -24i32..=24 {
                probe_left(j, ib, t + k as f64 * b, top);
            }
        }
    }
}

fn join_nk(j: &J, top: &mut Best, pairs: &[(u64, u64)]) {
    let mut bl = [0u32; 3];
    let mut br = [0u32; 3];
    for &(n, k) in pairs {
        let nl = j.l.find(n as f64, &mut bl);
        if nl == 0 {
            continue;
        }
        let nr = j.r.find(k as f64, &mut br);
        for &ia in &bl[..nl] {
            for &ib in &br[..nr] {
                try_pair(j, ia as usize, ib as usize, top);
            }
        }
    }
}

fn run_job(num: &[u8], cache: &Cache, prep: &RootPrep, ctx: &Ctx, pi: usize, split: usize, op: u8, top: &mut Best) {
    let cfg = ctx.cfg;
    let p = &prep.pts[pi];
    let l = cache.get(&num[..split]);
    let r = cache.get(&num[split..]);
    let w = cfg.w.op(op);
    if p.cost > top.bound() || l.min_cost() + r.min_cost() + w + p.cost > top.bound() {
        return;
    }
    let j = J { l, r, split: split as u8, op, w, pi: pi as u16, p, lim: &cfg.lim, tol: cfg.tol, target: cfg.target };
    match op {
        ADD | SUB | MUL | DIV | POW => join_invertible(&j, top),
        MOD => join_mod(&j, top),
        COMB => join_nk(&j, top, &prep.comb[pi]),
        PERM => join_nk(&j, top, &prep.perm[pi]),
        _ => {}
    }
}

/// The cheapest verified solution for one number, plus its alternatives.
/// `parallel` splits the (pretarget, split, op) jobs across threads, sharing the bound;
/// used when only a few numbers remain so per-number parallelism is not enough.
pub fn root_join(num: &[u8], cache: &Cache, prep: &RootPrep, ctx: &Ctx, parallel: bool) -> Option<Found> {
    let cfg = ctx.cfg;
    let shared = AtomicU32::new(u32::MAX);
    let new_top = || Best { budget: ctx.round.c_total, shared: &shared, found: Collector::new(cfg.slack), num, cache, prep, cfg };
    let mut top = new_top();
    let lit = parse_digits(num);
    let lit_cost = cfg.w.literal(num);
    let n = num.len();
    let whole = literal_allowed(num, ctx.round.concat);
    for (pi, p) in prep.pts.iter().enumerate().filter(|_| whole) {
        if let Some(v) = apply_chain(lit, &p.chain) {
            if (v - cfg.target).abs() <= cfg.tol {
                let c = Cand { score: mk_score(lit_cost + p.cost, n as u32 + p.len), split: n as u8, op: LIT, ia: 0, ib: 0, pt: pi as u16 };
                top.push(c);
            }
        }
    }
    if n == 1 {
        // a single digit has no binary split: look the target up in its own set
        let set = cache.get(num);
        let mut buf = [0u32; 3];
        let found = set.find(cfg.target, &mut buf);
        for &i in &buf[..found] {
            if (set.val[i as usize] - cfg.target).abs() <= cfg.tol {
                top.push(Cand { score: set.score[i as usize], split: 1, op: SET_ENTRY, ia: i, ib: 0, pt: 0 });
            }
        }
        return top.found.finish();
    }
    // pretargets in cost order first, so cheap solutions tighten the bound early
    let jobs: Vec<(usize, usize, u8)> = (0..prep.pts.len())
        .flat_map(|pi| (1..n).flat_map(move |split| ctx.binops.iter().map(move |&op| (pi, split, op))))
        .collect();
    if parallel {
        let parts: Vec<Collector> = jobs
            .par_iter()
            .with_max_len(1)
            .map(|&(pi, split, op)| {
                let mut t = new_top();
                run_job(num, cache, prep, ctx, pi, split, op, &mut t);
                t.found
            })
            .collect();
        for f in parts {
            top.found.merge(f);
        }
    } else {
        for &(pi, split, op) in &jobs {
            run_job(num, cache, prep, ctx, pi, split, op, &mut top);
        }
    }
    top.found.finish()
}
