//! Flat tiers: every tier before brackets are allowed, solved by direct enumeration.
//!
//! A flat expression is `[-]x1 op x2 op ... xk` where the x's are the digits (or
//! concatenated groups) and the ops are the round's binary operators among
//! + - * / ^ %. It is evaluated with the game's precedence: `^` right-associative first,
//! a leading minus applies to the first power tower (`-2^2 = -4`), then * / %
//! left-to-right, then + - left-to-right. Evaluation is numeric; an expression tree is
//! only built for candidates that beat the current best.

use crate::config::{Config, Round};
use crate::collect::{Collector, Found};
use crate::expr::Expr;
use crate::ops::*;
use crate::span::{mk_score, parse_digits};

const MAXD: usize = 6;

#[inline]
fn is_mul_level(op: u8) -> bool {
    matches!(op, MUL | DIV | MOD)
}

/// Value of a flat expression (None = undefined or non-finite).
fn eval_flat(v: &[f64], o: &[u8], neg: bool) -> Option<f64> {
    let k = v.len();
    let mut factors = [0f64; MAXD];
    let mut fops = [0u8; MAXD];
    let mut nf = 0;
    let mut i = 0;
    while i < k {
        let mut j = i;
        while j < k - 1 && o[j] == POW {
            j += 1;
        }
        let mut val = v[j];
        let mut m = j;
        while m > i {
            m -= 1;
            val = raw_bin(POW, v[m], val)?;
        }
        if nf > 0 {
            fops[nf - 1] = o[i - 1];
        }
        factors[nf] = val;
        nf += 1;
        i = j + 1;
    }
    if neg {
        factors[0] = -factors[0];
    }
    let mut acc_terms = 0f64;
    let mut have_acc = false;
    let mut pending = ADD;
    let mut cur = factors[0];
    for f in 1..nf {
        let op = fops[f - 1];
        if is_mul_level(op) {
            cur = raw_bin(op, cur, factors[f])?;
        } else {
            acc_terms = if have_acc { raw_bin(pending, acc_terms, cur)? } else { cur };
            have_acc = true;
            pending = op;
            cur = factors[f];
        }
    }
    if have_acc {
        raw_bin(pending, acc_terms, cur)
    } else {
        Some(cur).filter(|x| x.is_finite())
    }
}

/// The same expression as a tree (mirrors `eval_flat` exactly).
fn build_flat(groups: &[&[u8]], o: &[u8], neg: bool) -> Expr {
    let k = groups.len();
    let mut factors: Vec<Expr> = vec![];
    let mut fops: Vec<u8> = vec![];
    let mut i = 0;
    while i < k {
        let mut j = i;
        while j < k - 1 && o[j] == POW {
            j += 1;
        }
        let mut e = Expr::lit(groups[j]);
        let mut m = j;
        while m > i {
            m -= 1;
            e = Expr::bin(POW, Expr::lit(groups[m]), e);
        }
        if !factors.is_empty() {
            fops.push(o[i - 1]);
        }
        factors.push(e);
        i = j + 1;
    }
    let mut it = factors.into_iter();
    let mut cur = it.next().unwrap();
    if neg {
        cur = Expr::un(NEG, cur);
    }
    let mut acc: Option<Expr> = None;
    let mut pending = ADD;
    for (f, op) in it.zip(fops) {
        if is_mul_level(op) {
            cur = Expr::bin(op, cur, f);
        } else {
            acc = Some(match acc {
                None => cur,
                Some(a) => Expr::bin(pending, a, cur),
            });
            pending = op;
            cur = f;
        }
    }
    match acc {
        None => cur,
        Some(a) => Expr::bin(pending, a, cur),
    }
}

pub fn solve_flat(num: &[u8], round: &Round, cfg: &Config) -> Option<Found> {
    let ops: Vec<u8> = round.binops.iter().copied().filter(|&o| matches!(o, ADD | SUB | MUL | DIV | POW | MOD)).collect();
    let w = &cfg.w;
    let n = num.len();
    let mut found = Collector::new(cfg.slack);
    let full = (1u32 << (n - 1)) - 1; // cut between every pair of digits
    let masks: Vec<u32> = if round.concat { (0..=full).collect() } else { vec![full] };
    let mut vals = [0f64; MAXD];
    let mut o = [0u8; MAXD];
    for mask in masks {
        // bit i set = cut after digit i
        let mut groups: Vec<&[u8]> = Vec::with_capacity(n);
        let mut start = 0;
        for i in 0..n - 1 {
            if mask >> i & 1 == 1 {
                groups.push(&num[start..=i]);
                start = i + 1;
            }
        }
        groups.push(&num[start..]);
        if !groups.iter().all(|g| literal_allowed(g, round.concat)) {
            continue;
        }
        let k = groups.len();
        if k > 1 && ops.is_empty() {
            continue;
        }
        for (i, g) in groups.iter().enumerate() {
            vals[i] = parse_digits(g);
        }
        let lit_cost: u32 = groups.iter().map(|g| w.literal(g)).sum();
        let combos = ops.len().pow(k as u32 - 1).max(1);
        for code in 0..combos {
            let mut c = code;
            let mut op_cost = 0;
            for slot in o.iter_mut().take(k - 1) {
                *slot = ops[c % ops.len()];
                c /= ops.len();
                op_cost += w.op(*slot);
            }
            for neg in [false, true] {
                if neg && (!round.neg || vals[0] == 0.0) {
                    continue;
                }
                let cost = lit_cost + op_cost + if neg { w.op(NEG) } else { 0 };
                if cost > found.cost_limit(found.best_score()) {
                    continue;
                }
                let Some(v) = eval_flat(&vals[..k], &o[..k - 1], neg) else { continue };
                if (v - cfg.target).abs() > cfg.tol {
                    continue;
                }
                let e = build_flat(&groups, &o[..k - 1], neg);
                let text = e.render();
                let sc = mk_score(cost, text.len() as u32);
                let best = found.best_score();
                if found.wants(sc, &text, best) && e.verify_exact(&cfg.target_str) {
                    found.add(sc, text, e, best);
                }
            }
        }
    }
    found.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_precedence_matches_tree() {
        let groups: Vec<&[u8]> = vec![b"2", b"3", b"2", b"4"];
        for ops in [[POW, POW, SUB], [SUB, POW, MUL], [MUL, MOD, ADD], [DIV, SUB, POW]] {
            for neg in [false, true] {
                let v = eval_flat(&[2.0, 3.0, 2.0, 4.0], &ops, neg);
                let e = build_flat(&groups, &ops, neg);
                assert_eq!(v, e.eval(), "{}", e.render());
                assert!(!e.render_info().1, "flat expression rendered brackets: {}", e.render());
            }
        }
        // -2^3^2 = -(2^9)
        assert_eq!(eval_flat(&[2.0, 3.0, 2.0], &[POW, POW], true), Some(-512.0));
    }
}
