//! Value sets for digit substrings.
//!
//! A `SpanSet` holds, for one digit substring, every reachable value (deduplicated by
//! quantised key) together with its cheapest expression. Entries are stored in
//! structure-of-arrays form, sorted by score (cost, then rendered length), and each
//! entry *is* its expression node: unary nodes point at another entry of the same set,
//! binary nodes point into the (already frozen) sets of the two child substrings.

use crate::config::{Config, Round};
use crate::key::{hkey, key};
use crate::ops::*;
use hashbrown::HashTable;

pub const NONE: u32 = u32::MAX;
pub const INF_COST: u32 = 1 << 19;

#[derive(Clone, Copy, Default, Debug)]
pub struct Node {
    pub op: u8,
    pub split: u8,
    pub a: u32,
    pub b: u32,
}

#[inline]
pub fn mk_score(cost: u32, len: u32) -> u32 {
    (cost.min(INF_COST) << 12) | len.min(4095)
}

/// Rendered length of a binary node, mirroring `expr::Expr::render` paren rules.
#[inline]
pub fn bin_len(op: u8, la: u32, opa: u8, lb: u32, opb: u8) -> u32 {
    match op {
        COMB | PERM => la + lb + 16,
        POW => la + lb + 1 + if prec(opa) <= 4 { 2 } else { 0 } + if prec(opb) < 4 { 2 } else { 0 },
        _ => {
            let p = prec(op);
            la + lb + 3 + if prec(opa) < p { 2 } else { 0 } + if prec(opb) <= p || opb == NEG { 2 } else { 0 }
        }
    }
}

/// Rendered length of a unary node.
#[inline]
pub fn un_len(op: u8, lc: u32, opc: u8) -> u32 {
    match op {
        NEG => lc + 1 + if prec(opc) < 3 || opc == NEG { 2 } else { 0 },
        FACT => lc + 1 + if prec(opc) < 5 { 2 } else { 0 },
        _ if is_macro(op) => {
            let (o, i) = macro_parts(op);
            lc + (name(o).len() + name(i).len()) as u32 + 4
        }
        _ => lc + name(op).len() as u32 + 2,
    }
}

pub struct SpanSet {
    pub val: Vec<f64>,
    pub score: Vec<u32>,
    pub flags: Vec<u8>,
    pub node: Vec<Node>,
    /// cost_end[c] = number of entries with cost <= c
    cost_end: Vec<u32>,
    index: HashTable<u32>,
    /// entries costlier than this may be missing (set hit `max_entries`)
    pub truncated_at: Option<u32>,
    pub candidates: u64,
}

impl SpanSet {
    pub fn empty() -> Self {
        SpanSet {
            val: vec![],
            score: vec![],
            flags: vec![],
            node: vec![],
            cost_end: vec![],
            index: HashTable::new(),
            truncated_at: None,
            candidates: 0,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.val.len()
    }
    #[inline]
    pub fn cost(&self, i: usize) -> u32 {
        self.score[i] >> 12
    }
    #[inline]
    pub fn rlen(&self, i: usize) -> u32 {
        self.score[i] & 0xFFF
    }
    #[inline]
    pub fn min_cost(&self) -> u32 {
        if self.val.is_empty() {
            INF_COST
        } else {
            self.cost(0)
        }
    }
    /// Number of entries with cost <= c (entries are sorted by cost).
    #[inline]
    pub fn count_le(&self, c: u32) -> usize {
        let c = c as usize;
        if c >= self.cost_end.len() {
            self.val.len()
        } else {
            self.cost_end[c] as usize
        }
    }
    /// Entries whose key is within ±1 of `v`'s key. Returns how many were written.
    #[inline]
    pub fn find(&self, v: f64, out: &mut [u32; 3]) -> usize {
        if !v.is_finite() {
            return 0;
        }
        let k = key(v);
        let mut n = 0;
        for kk in [k.wrapping_sub(1), k, k.wrapping_add(1)] {
            if let Some(&i) = self.index.find(hkey(kk), |&i| key(self.val[i as usize]) == kk) {
                out[n] = i;
                n += 1;
            }
        }
        n
    }
    pub fn mem_bytes(&self) -> usize {
        self.len() * (8 + 4 + 1 + 12 + 5)
    }
}

/// Everything a set build needs to know about the current round.
pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub round: &'a Round,
    pub binops: Vec<u8>,
    pub unops: Vec<u8>,
}

impl<'a> Ctx<'a> {
    pub fn new(cfg: &'a Config, round: &'a Round) -> Self {
        Ctx { cfg, round, binops: round.binops.clone(), unops: round.unops.clone() }
    }
}

struct Builder {
    val: Vec<f64>,
    score: Vec<u32>,
    flags: Vec<u8>,
    node: Vec<Node>,
    fin: Vec<u32>,
    table: HashTable<u32>,
    cap: u32,
    closure: bool,
    buckets: Vec<Vec<u32>>,
    truncated_at: Option<u32>,
    max_entries: usize,
    next_check: usize,
    candidates: u64,
}

impl Builder {
    fn new(cap: u32, max_entries: usize) -> Self {
        Builder {
            val: Vec::new(),
            score: Vec::new(),
            flags: Vec::new(),
            node: Vec::new(),
            fin: Vec::new(),
            table: HashTable::new(),
            cap,
            closure: false,
            buckets: Vec::new(),
            truncated_at: None,
            max_entries,
            next_check: max_entries.saturating_mul(2).max(1 << 16),
            candidates: 0,
        }
    }

    #[inline]
    fn insert(&mut self, v: f64, f: u8, score: u32, node: Node) {
        self.candidates += 1;
        let cost = score >> 12;
        if cost > self.cap {
            return;
        }
        let k = key(v);
        let h = hkey(k);
        let val = &self.val;
        if let Some(&s) = self.table.find(h, |&s| key(val[s as usize]) == k) {
            let s = s as usize;
            let better = score < self.score[s] || (score == self.score[s] && (f & EXACT) > (self.flags[s] & EXACT));
            if better && self.fin[s] == NONE {
                self.val[s] = v;
                self.score[s] = score;
                self.flags[s] = f;
                self.node[s] = node;
                if self.closure {
                    self.push_bucket(cost, s as u32);
                }
            }
            return;
        }
        let s = self.val.len() as u32;
        self.val.push(v);
        self.score.push(score);
        self.flags.push(f);
        self.node.push(node);
        self.fin.push(NONE);
        let val = &self.val;
        self.table.insert_unique(h, s, |&x| hkey(key(val[x as usize])));
        if self.closure {
            self.push_bucket(cost, s);
        } else if self.val.len() >= self.next_check {
            self.shrink_cap();
        }
    }

    #[inline]
    fn push_bucket(&mut self, cost: u32, s: u32) {
        let c = cost as usize;
        if c >= self.buckets.len() {
            self.buckets.resize_with(c + 1, Vec::new);
        }
        self.buckets[c].push(s);
    }

    /// Binary phase grew too large: lower the cost cap so that roughly `max_entries`
    /// of the cheapest values survive.
    fn shrink_cap(&mut self) {
        let mut hist: Vec<usize> = Vec::new();
        for &sc in &self.score {
            let c = (sc >> 12) as usize;
            if c >= hist.len() {
                hist.resize(c + 1, 0);
            }
            hist[c] += 1;
        }
        let mut acc = 0;
        for (c, &n) in hist.iter().enumerate() {
            acc += n;
            if acc >= self.max_entries {
                let c = c as u32;
                if c < self.cap {
                    self.cap = c;
                    self.truncated_at = Some(self.truncated_at.map_or(c, |t| t.min(c)));
                }
                break;
            }
        }
        self.next_check = self.val.len() + self.max_entries.max(1 << 16);
    }

    fn expand(&mut self, s: usize, ctx: &Ctx, out_len: u32) {
        let v = self.val[s];
        let f = self.flags[s];
        let op_c = self.node[s].op;
        let c = self.score[s] >> 12;
        let lim = &ctx.cfg.lim;
        for &u in &ctx.unops {
            let w = ctx.cfg.w.op(u);
            if c + w > self.cap {
                continue;
            }
            // skip applications that cannot help
            let skip = match u {
                NEG => v == 0.0 || op_c == NEG,
                FACT => v == 1.0 || v == 2.0,
                SQRT => v == 0.0 || v == 1.0,
                CBRT => v == 0.0 || v.abs() == 1.0,
                _ if is_fwd_trig(u) => !is_pirat(v),
                _ => false,
            };
            if skip {
                continue;
            }
            if let Some((r, rf)) = un(u, v, f, lim) {
                if ctx.round.pi_filter && is_inv_trig(u) && !is_pirat(r) {
                    continue;
                }
                let len = un_len(u, out_len, op_c);
                self.insert(r, rf, mk_score(c + w, len), Node { op: u, split: 0, a: s as u32, b: 0 });
            }
        }
    }

    fn finish(mut self, ctx: &Ctx) -> SpanSet {
        // ---- unary closure in cost order (Dial buckets)
        self.closure = true;
        for s in 0..self.val.len() {
            let c = self.score[s] >> 12;
            if c <= self.cap {
                self.push_bucket(c, s as u32);
            }
        }
        let n0 = self.val.len();
        let mut val = Vec::with_capacity(n0);
        let mut score = Vec::with_capacity(n0);
        let mut flags = Vec::with_capacity(n0);
        let mut node: Vec<Node> = Vec::with_capacity(n0);
        let mut c = 0usize;
        while c < self.buckets.len() && c as u32 <= self.cap {
            let bucket = std::mem::take(&mut self.buckets[c]);
            for &s in &bucket {
                let s = s as usize;
                if self.fin[s] != NONE || (self.score[s] >> 12) as usize != c {
                    continue; // stale
                }
                self.fin[s] = val.len() as u32;
                let mut nd = self.node[s];
                if is_unary(nd.op) {
                    nd.a = self.fin[nd.a as usize];
                }
                val.push(self.val[s]);
                score.push(self.score[s]);
                flags.push(self.flags[s]);
                node.push(nd);
                self.expand(s, ctx, self.score[s] & 0xFFF);
            }
            if val.len() >= self.max_entries {
                let t = c as u32;
                self.truncated_at = Some(self.truncated_at.map_or(t, |x| x.min(t)));
                break;
            }
            c += 1;
        }
        // ---- freeze
        let maxc = score.last().map_or(0, |s| s >> 12) as usize;
        let mut cost_end = vec![0u32; maxc + 1];
        let mut i = 0usize;
        for (cc, slot) in cost_end.iter_mut().enumerate() {
            while i < score.len() && (score[i] >> 12) as usize <= cc {
                i += 1;
            }
            *slot = i as u32;
        }
        let mut index = HashTable::with_capacity(val.len());
        for (i, &v) in val.iter().enumerate() {
            index.insert_unique(hkey(key(v)), i as u32, |&x: &u32| hkey(key(val[x as usize])));
        }
        SpanSet {
            val,
            score,
            flags,
            node,
            cost_end,
            index,
            truncated_at: self.truncated_at,
            candidates: self.candidates,
        }
    }
}

pub fn parse_digits(d: &[u8]) -> f64 {
    d.iter().fold(0u64, |a, &c| a * 10 + (c - b'0') as u64) as f64
}

#[inline(always)]
fn combine_with<F: Fn(f64, u8, f64, u8) -> Option<(f64, u8)>>(
    b: &mut Builder,
    l: &SpanSet,
    r: &SpanSet,
    op: u8,
    split: u8,
    w: u32,
    f: F,
) {
    let rmin = r.min_cost();
    for ia in 0..l.len() {
        let ca = l.cost(ia);
        if ca + w + rmin > b.cap {
            break;
        }
        let end = r.count_le(b.cap - ca - w);
        let (x, fx, la, opa) = (l.val[ia], l.flags[ia], l.rlen(ia), l.node[ia].op);
        for ib in 0..end {
            if let Some((v, fv)) = f(x, fx, r.val[ib], r.flags[ib]) {
                let cost = ca + w + r.cost(ib);
                let len = bin_len(op, la, opa, r.rlen(ib), r.node[ib].op);
                b.insert(v, fv, mk_score(cost, len), Node { op, split, a: ia as u32, b: ib as u32 });
            }
        }
    }
}

fn combine(b: &mut Builder, l: &SpanSet, r: &SpanSet, op: u8, split: u8, ctx: &Ctx) {
    let lim = ctx.cfg.lim;
    let w = ctx.cfg.w.op(op);
    match op {
        ADD => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_add(x, fx, y, fy, &lim)),
        SUB => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_sub(x, fx, y, fy, &lim)),
        MUL => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_mul(x, fx, y, fy, &lim)),
        DIV => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_div(x, fx, y, fy, &lim)),
        POW => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_pow(x, fx, y, fy, &lim)),
        MOD => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_mod(x, fx, y, fy, &lim)),
        COMB | PERM => combine_with(b, l, r, op, split, w, |x, fx, y, fy| bin_comb_perm(op, x, fx, y, fy, &lim)),
        _ => unreachable!(),
    }
}

/// Build the value set of `digits` from the already-built sets of its sub-substrings.
pub fn build_set<'s>(digits: &[u8], get: &dyn Fn(&[u8]) -> &'s SpanSet, ctx: &Ctx) -> SpanSet {
    // the other side of any enclosing binary op costs >= 0 and the op itself >= 10
    let cap = ctx.round.c_total.saturating_sub(10);
    let mut b = Builder::new(cap, ctx.round.max_entries);
    if literal_allowed(digits, ctx.round.concat) {
        b.insert(
            parse_digits(digits),
            EXACT | INT,
            mk_score(ctx.cfg.w.literal(digits), digits.len() as u32),
            Node { op: LIT, split: 0, a: 0, b: 0 },
        );
    }
    for split in 1..digits.len() {
        let l = get(&digits[..split]);
        let r = get(&digits[split..]);
        for &op in &ctx.binops {
            combine(&mut b, l, r, op, split as u8, ctx);
        }
    }
    b.finish(ctx)
}
