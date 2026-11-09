//! Pretargets: values `t` plus a unary chain `u_k(...u_1(t))` that evaluates to the
//! target.
//!
//! The root of a solution is `chain(a op b)`, so the root join looks for `a op b = t`
//! for each pretarget. Computed by inverting the round's unary ops starting from the
//! target,
//! with every candidate verified by evaluating the chain forwards.

use crate::config::{Config, Round};
use crate::key::key;
use crate::ops::*;

#[derive(Clone, Debug)]
pub struct Pretarget {
    pub t: f64,
    pub cost: u32,
    /// extra rendered length of the chain
    pub len: u32,
    /// ops applied to t, innermost first
    pub chain: Vec<u8>,
}

pub fn apply_chain(x: f64, chain: &[u8]) -> Option<f64> {
    let mut v = x;
    for &u in chain {
        v = raw_un(u, v)?;
    }
    Some(v)
}

fn macro_inverse(m: u8, y: f64) -> f64 {
    let s = f64::sqrt;
    match m - MACRO0 {
        0 => s(y * y - 1.0),         // sec(atan x) = sqrt(1+x^2)
        1 => 1.0 / y,                // cot(atan x) = 1/x
        2 => s(y * y + 1.0),         // tan(asec x) = ±sqrt(x^2-1)
        3 => y / s(1.0 - y * y),     // sin(atan x) = x/sqrt(1+x^2)
        4 => s(1.0 / (y * y) - 1.0), // cos(atan x) = 1/sqrt(1+x^2)
        5 => 1.0 / s(y * y - 1.0),   // csc(atan x) = sqrt(1+x^2)/x
        6 => s(1.0 - y * y),         // sin(acos x) = sqrt(1-x^2)
        7 => 1.0 / s(1.0 + y * y),   // tan(acos x) = sqrt(1-x^2)/x
        8 => y / s(1.0 + y * y),     // tan(asin x) = x/sqrt(1-x^2)
        9 => s(1.0 - 1.0 / (y * y)), // sec(asin x) = 1/sqrt(1-x^2)
        10 => 1.0 / s(1.0 - y * y),  // sin(asec x) = sqrt(1-1/x^2)
        11 => s(1.0 + 1.0 / (y * y)), // cot(asec x) = ±1/sqrt(x^2-1)
        _ => y / s(y * y - 1.0),     // csc(asec x) = |x|/sqrt(x^2-1)
    }
}

fn inverse_candidates(u: u8, y: f64, lim: &Limits) -> Vec<f64> {
    match u {
        NEG => vec![-y],
        SQRT if y > 0.0 => vec![y * y],
        CBRT if y.abs() > 1.0 => vec![y * y * y],
        FACT if y >= 6.0 && y == y.round() => {
            let mut v = vec![];
            let mut n = 3u32;
            while (n as f64) <= lim.fact_max {
                if fact_int(n) == y {
                    v.push(n as f64);
                }
                n += 1;
            }
            v
        }
        _ if is_macro(u) => {
            let c = macro_inverse(u, y);
            vec![c, -c]
        }
        _ => vec![],
    }
}

pub fn pretargets(round: &Round, cfg: &Config) -> Vec<Pretarget> {
    let unops = &round.unops;
    let lim = &cfg.lim;
    let mut out = vec![Pretarget { t: cfg.target, cost: 0, len: 0, chain: vec![] }];
    let mut expanded = vec![false];
    loop {
        // expand the cheapest unexpanded pretarget
        let Some(i) = (0..out.len()).filter(|&i| !expanded[i]).min_by_key(|&i| out[i].cost) else {
            break;
        };
        expanded[i] = true;
        let p = out[i].clone();
        for &u in unops {
            let cost = p.cost + cfg.w.op(u);
            if cost > round.root_budget {
                continue;
            }
            for x in inverse_candidates(u, p.t, lim) {
                if !x.is_finite() || x.abs() > lim.max_abs || (x != 0.0 && x.abs() < lim.min_abs) {
                    continue;
                }
                if p.chain.first() == Some(&NEG) && u == NEG {
                    continue;
                }
                let mut chain = vec![u];
                chain.extend_from_slice(&p.chain);
                let Some(v) = apply_chain(x, &chain) else { continue };
                if (v - cfg.target).abs() > 0.1 * cfg.tol {
                    continue;
                }
                if out.iter().any(|q| key(q.t) == key(x)) {
                    continue;
                }
                let len = p.len + crate::span::un_len(u, 0, ADD);
                out.push(Pretarget { t: x, cost, len, chain });
                expanded.push(false);
            }
        }
        if out.len() >= 256 {
            break;
        }
    }
    out.sort_by_key(|p| (p.cost, p.len));
    out.truncate(64);
    out
}
