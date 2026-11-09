//! Random expression generator for cross-checking rendering + semantics against mathjs
//! (`node verify/exprcheck.mjs output/expr_tests.json`).

use crate::expr::Expr;
use crate::ops::*;
use serde::Serialize;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const UNARY: [u8; 29] = [
    NEG, NEG, NEG, FACT, FACT, SQRT, SQRT, CBRT, SIN, COS, TAN, SEC, CSC, COT, ASIN, ACOS, ATAN, ASEC, ACSC, ACOT,
    MACRO0, MACRO0 + 1, MACRO0 + 2, MACRO0 + 3, MACRO0 + 4, MACRO0 + 6, MACRO0 + 8, MACRO0 + 10, MACRO0 + 12,
];
const BINARY: [u8; 10] = [ADD, SUB, MUL, DIV, POW, POW, MOD, MOD, COMB, PERM];

fn gen(rng: &mut Rng, depth: u32) -> Expr {
    if depth == 0 || rng.below(5) == 0 {
        let len = 1 + (rng.below(10) / 8) as usize; // mostly single digits
        let s: String = (0..len).map(|_| (b'0' + rng.below(10) as u8) as char).collect();
        return Expr::Lit(s);
    }
    if rng.below(5) < 2 {
        let op = UNARY[rng.below(UNARY.len() as u64) as usize];
        Expr::un(op, gen(rng, depth - 1))
    } else {
        let op = BINARY[rng.below(BINARY.len() as u64) as usize];
        Expr::bin(op, gen(rng, depth - 1), gen(rng, depth - 1))
    }
}

#[derive(Serialize)]
pub struct ExprTest {
    pub expr: String,
    pub value: Option<f64>,
}

/// mathjs evaluates combinations/permutations with a loop of length k, so huge
/// arguments (e.g. combinations(9^9, 9^8)) would hang the checker: keep them small.
fn mathjs_friendly(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) => true,
        // sin/cos/tan of a huge argument is chaotic: 1 ulp of libm-vs-V8 difference in
        // the argument changes the result completely (the solver never does this)
        Expr::Un(op, c) => !(is_fwd_trig(*op) && c.eval().is_some_and(|v| v.abs() > 1e6)) && mathjs_friendly(c),
        Expr::Bin(op, a, b) => {
            // mod with a huge quotient is ill-conditioned (ulp differences get amplified)
            if *op == MOD {
                if let (Some(x), Some(y)) = (a.eval(), b.eval()) {
                    if y != 0.0 && (x / y).abs() > 1e6 {
                        return false;
                    }
                }
            }
            if matches!(*op, COMB | PERM) {
                for x in [a, b] {
                    // an argument Rust rejects may still be huge (and slow) in mathjs
                    if x.eval().is_none_or(|v| v.abs() > 200.0) {
                        return false;
                    }
                }
            }
            mathjs_friendly(a) && mathjs_friendly(b)
        }
    }
}

pub fn expr_tests(n: usize, seed: u64) -> Vec<ExprTest> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let depth = 1 + rng.below(4) as u32;
        let e = gen(&mut rng, depth);
        if mathjs_friendly(&e) {
            out.push(ExprTest { expr: e.render(), value: e.eval() });
        }
    }
    out
}
