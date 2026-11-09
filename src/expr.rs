//! Expression trees: reconstruction, minimal-paren mathjs rendering, f64 evaluation
//! (mathjs semantics) and 256-bit verification.

use crate::cache::Cache;
use crate::ops::*;
use crate::config::Feature;
use astro_float::{BigFloat, Consts, Radix, RoundingMode};

/// Render target that also records whether grouping brackets were emitted.
struct Out {
    s: String,
    grouped: bool,
}

impl Out {
    fn push(&mut self, c: char) {
        self.s.push(c);
    }
    fn push_str(&mut self, x: &str) {
        self.s.push_str(x);
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(String),
    Un(u8, Box<Expr>),
    Bin(u8, Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn lit(d: &[u8]) -> Expr {
        Expr::Lit(String::from_utf8(d.to_vec()).unwrap())
    }
    pub fn un(op: u8, c: Expr) -> Expr {
        if is_macro(op) {
            let (outer, inner) = macro_parts(op);
            Expr::Un(outer, Box::new(Expr::Un(inner, Box::new(c))))
        } else {
            Expr::Un(op, Box::new(c))
        }
    }
    pub fn bin(op: u8, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, Box::new(a), Box::new(b))
    }

    /// Rebuild the expression of entry `idx` of the set for `digits`.
    pub fn from_set(cache: &Cache, digits: &[u8], idx: u32) -> Expr {
        let nd = cache.get(digits).node[idx as usize];
        match nd.op {
            LIT => Expr::lit(digits),
            op if is_binary(op) => {
                let s = nd.split as usize;
                Expr::bin(op, Expr::from_set(cache, &digits[..s], nd.a), Expr::from_set(cache, &digits[s..], nd.b))
            }
            op => Expr::un(op, Expr::from_set(cache, digits, nd.a)),
        }
    }

    pub fn top_op(&self) -> u8 {
        match self {
            Expr::Lit(_) => LIT,
            Expr::Un(op, _) | Expr::Bin(op, _, _) => *op,
        }
    }
    fn prec(&self) -> u8 {
        prec(self.top_op())
    }

    pub fn render(&self) -> String {
        self.render_info().0
    }

    /// Rendered text, plus whether it needed any grouping brackets (as opposed to
    /// function-call parentheses). Grouping brackets are the "brackets" feature.
    pub fn render_info(&self) -> (String, bool) {
        let mut out = Out { s: String::new(), grouped: false };
        self.write(&mut out, true);
        (out.s, out.grouped)
    }

    fn write_paren(&self, out: &mut Out, paren: bool, at_start: bool) {
        if paren {
            out.grouped = true;
            out.push('(');
            self.write(out, true);
            out.push(')');
        } else {
            self.write(out, at_start);
        }
    }

    /// `at_start`: this subexpression begins the whole expression or follows `(` / `,`.
    /// A unary minus anywhere else is parenthesised for readability (`6 + (-17) % 7`).
    fn write(&self, out: &mut Out, at_start: bool) {
        match self {
            Expr::Lit(s) => out.push_str(s),
            Expr::Un(NEG, c) => {
                if !at_start {
                    out.grouped = true;
                    out.push('(');
                }
                out.push('-');
                c.write_paren(out, c.prec() < 3 || c.top_op() == NEG, false);
                if !at_start {
                    out.push(')');
                }
            }
            Expr::Un(FACT, c) => {
                c.write_paren(out, c.prec() < 5, at_start);
                out.push('!');
            }
            Expr::Un(op, c) => {
                out.push_str(name(*op));
                out.push('(');
                c.write(out, true);
                out.push(')');
            }
            Expr::Bin(op @ (COMB | PERM), a, b) => {
                out.push_str(name(*op));
                out.push('(');
                a.write(out, true);
                out.push_str(", ");
                b.write(out, true);
                out.push(')');
            }
            Expr::Bin(POW, a, b) => {
                a.write_paren(out, a.prec() <= 4, at_start);
                out.push('^');
                b.write_paren(out, b.prec() < 4, false);
            }
            Expr::Bin(op, a, b) => {
                let p = prec(*op);
                a.write_paren(out, a.prec() < p, at_start);
                out.push(' ');
                out.push_str(name(*op));
                out.push(' ');
                b.write_paren(out, b.prec() <= p, false);
            }
        }
    }

    /// mathjs-semantics f64 evaluation. None = invalid in mathjs (or non-finite).
    pub fn eval(&self) -> Option<f64> {
        match self {
            Expr::Lit(s) => s.parse::<f64>().ok(),
            Expr::Un(op, c) => raw_un(*op, c.eval()?),
            Expr::Bin(op, a, b) => raw_bin(*op, a.eval()?, b.eval()?),
        }
    }

    /// Bitmask of the features (see `config::Feature`) this expression uses.
    pub fn features(&self) -> u32 {
        fn walk(e: &Expr) -> u32 {
            match e {
                Expr::Lit(s) => {
                    if s.len() > 1 {
                        Feature::Concat.bit()
                    } else {
                        0
                    }
                }
                Expr::Un(op, c) => Feature::of_op(*op).map_or(0, |f| f.bit()) | walk(c),
                Expr::Bin(op, a, b) => Feature::of_op(*op).map_or(0, |f| f.bit()) | walk(a) | walk(b),
            }
        }
        let grouped = self.render_info().1;
        walk(self) | if grouped { Feature::Brackets.bit() } else { 0 }
    }

    /// Exact check: evaluate with 256-bit floats and require |v - target| < 2^-200
    /// (`target` is the exact decimal string). Rejects coincidental f64 near-misses.
    pub fn verify_exact(&self, target: &str) -> bool {
        let mut cc = match Consts::new() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let t = BigFloat::parse(target, Radix::Dec, P, RM, &mut cc);
        if t.is_nan() {
            return false;
        }
        match hp_eval(self, &mut cc) {
            Some((_, v)) => {
                let d = v.sub(&t, P, RM).abs();
                let eps = BigFloat::from_u8(1, P).div(&BigFloat::from_u8(2, P).powi(200, P, RM), P, RM);
                matches!(d.cmp(&eps), Some(c) if c < 0)
            }
            None => false,
        }
    }
}

const P: usize = 256;
const RM: RoundingMode = RoundingMode::ToEven;

fn ok(b: BigFloat) -> Option<BigFloat> {
    if b.is_nan() || b.is_inf() {
        None
    } else {
        Some(b)
    }
}

fn one() -> BigFloat {
    BigFloat::from_u8(1, P)
}

/// Small non-negative integer value of a high-precision number, if it is one.
fn small_int(f: f64, b: &BigFloat) -> Option<u64> {
    if !(0.0..=1e15).contains(&f) || f != f.round() {
        return None;
    }
    let n = f as u64;
    if BigFloat::from_u64(n, P).cmp(b) == Some(0) {
        Some(n)
    } else {
        None
    }
}

/// Every node's f64 value (what mathjs computes) must agree with its 256-bit value to
/// 1e-9 relative (scaled by the operands, so cancellation noise is tolerated). Catches
/// float artefacts the final answer alone can't reveal, e.g. 7^(4e-4810) == 7^0 in
/// floats, or a tolerant floor in `%` that differs from the exact one.
fn consistent(rf: f64, r: BigFloat, operand_scale: f64) -> Option<(f64, BigFloat)> {
    let scale = 1f64.max(rf.abs()).max(operand_scale);
    let d = r.sub(&BigFloat::from_f64(rf, P), P, RM).abs();
    match d.cmp(&BigFloat::from_f64(1e-9 * scale, P)) {
        Some(c) if c <= 0 => Some((rf, r)),
        _ => None,
    }
}

/// Evaluate in f64 (mathjs semantics, for branch decisions) and 256-bit in parallel.
fn hp_eval(e: &Expr, cc: &mut Consts) -> Option<(f64, BigFloat)> {
    match e {
        Expr::Lit(s) => Some((s.parse().ok()?, BigFloat::from_u64(s.parse().ok()?, P))),
        Expr::Un(op, c) => {
            let (xf, x) = hp_eval(c, cc)?;
            let rf = raw_un(*op, xf)?;
            let r = match *op {
                NEG => x.neg(),
                FACT => {
                    let n = small_int(xf, &x)?;
                    if n > 170 {
                        return None;
                    }
                    let mut acc = one();
                    for i in 2..=n {
                        acc = acc.mul(&BigFloat::from_u64(i, P), P, RM);
                    }
                    acc
                }
                SQRT => {
                    if x.is_negative() {
                        return None;
                    }
                    x.sqrt(P, RM)
                }
                CBRT => {
                    if x.is_negative() {
                        x.neg().cbrt(P, RM).neg()
                    } else {
                        x.cbrt(P, RM)
                    }
                }
                SIN => x.sin(P, RM, cc),
                COS => x.cos(P, RM, cc),
                TAN => x.tan(P, RM, cc),
                SEC => one().div(&x.cos(P, RM, cc), P, RM),
                CSC => one().div(&x.sin(P, RM, cc), P, RM),
                COT => one().div(&x.tan(P, RM, cc), P, RM),
                ASIN | ACOS => {
                    if x.abs().cmp(&one()) == Some(1) {
                        return None;
                    }
                    if *op == ASIN {
                        x.asin(P, RM, cc)
                    } else {
                        x.acos(P, RM, cc)
                    }
                }
                ATAN => x.atan(P, RM, cc),
                ASEC | ACSC => {
                    if x.abs().cmp(&one()) == Some(-1) {
                        return None;
                    }
                    let inv = one().div(&x, P, RM);
                    if *op == ASEC {
                        inv.acos(P, RM, cc)
                    } else {
                        inv.asin(P, RM, cc)
                    }
                }
                ACOT => {
                    if x.is_zero() {
                        // atan(1/±0) = ±pi/2, sign from the f64 evaluation
                        let half_pi = cc.pi(P, RM).div(&BigFloat::from_u8(2, P), P, RM);
                        if xf.is_sign_negative() {
                            half_pi.neg()
                        } else {
                            half_pi
                        }
                    } else {
                        one().div(&x, P, RM).atan(P, RM, cc)
                    }
                }
                _ => return None,
            };
            consistent(rf, ok(r)?, xf.abs())
        }
        Expr::Bin(op, a, b) => {
            let (xf, x) = hp_eval(a, cc)?;
            let (yf, y) = hp_eval(b, cc)?;
            let rf = raw_bin(*op, xf, yf)?;
            // a zero product, quotient or power of nonzero operands is float underflow
            // (e.g. 9^(-7!) = 4e-4810): mathematically it is not 0
            let underflow = rf == 0.0
                && match *op {
                    MUL => xf != 0.0 && yf != 0.0,
                    DIV | POW => xf != 0.0,
                    _ => false,
                };
            if underflow {
                return None;
            }
            let r = match *op {
                ADD => x.add(&y, P, RM),
                SUB => x.sub(&y, P, RM),
                MUL => x.mul(&y, P, RM),
                DIV => {
                    if y.is_zero() {
                        return None;
                    }
                    x.div(&y, P, RM)
                }
                POW => {
                    if y.is_int() && yf.abs() <= 1e4 {
                        let n = yf.abs() as usize;
                        let p = x.powi(n, P, RM);
                        if yf < 0.0 {
                            if p.is_zero() {
                                return None;
                            }
                            one().div(&p, P, RM)
                        } else {
                            p
                        }
                    } else if x.is_positive() && !x.is_zero() {
                        x.pow(&y, P, RM, cc)
                    } else if x.is_zero() && y.is_positive() {
                        BigFloat::from_u8(0, P)
                    } else {
                        return None;
                    }
                }
                MOD => {
                    if y.is_zero() {
                        x
                    } else {
                        let q = x.div(&y, P, RM).floor();
                        x.sub(&y.mul(&q, P, RM), P, RM)
                    }
                }
                COMB | PERM => {
                    let n = small_int(xf, &x)?;
                    let k = small_int(yf, &y)?;
                    let v = comb_perm(*op, n as f64, k as f64)?;
                    BigFloat::from_u64(v as u64, P)
                }
                _ => return None,
            };
            consistent(rf, ok(r)?, xf.abs().max(yf.abs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(s: &str) -> Expr {
        Expr::Lit(s.to_string())
    }

    #[test]
    fn render_precedence() {
        // -2^2 stays bare, (-2)^2 needs parens
        assert_eq!(Expr::un(NEG, Expr::bin(POW, l("2"), l("2"))).render(), "-2^2");
        assert_eq!(Expr::bin(POW, Expr::un(NEG, l("2")), l("2")).render(), "(-2)^2");
        // right-assoc pow
        assert_eq!(Expr::bin(POW, l("2"), Expr::bin(POW, l("3"), l("2"))).render(), "2^3^2");
        assert_eq!(Expr::bin(POW, Expr::bin(POW, l("2"), l("3")), l("2")).render(), "(2^3)^2");
        // factorial
        assert_eq!(Expr::un(FACT, Expr::un(FACT, l("3"))).render(), "3!!");
        assert_eq!(Expr::un(FACT, Expr::bin(ADD, l("1"), l("2"))).render(), "(1 + 2)!");
        assert_eq!(Expr::bin(POW, l("2"), Expr::un(FACT, l("3"))).render(), "2^3!");
        // left-assoc subtraction and mod
        assert_eq!(Expr::bin(SUB, l("1"), Expr::bin(SUB, l("2"), l("3"))).render(), "1 - (2 - 3)");
        assert_eq!(Expr::bin(SUB, Expr::bin(SUB, l("1"), l("2")), l("3")).render(), "1 - 2 - 3");
        assert_eq!(Expr::bin(MOD, l("17"), Expr::un(NEG, l("7"))).render(), "17 % (-7)");
        assert_eq!(Expr::bin(MUL, l("2"), Expr::bin(MOD, l("7"), l("3"))).render(), "2 * (7 % 3)");
        assert_eq!(Expr::un(NEG, Expr::bin(MOD, l("7"), l("3"))).render(), "-(7 % 3)");
        // unary minus mid-expression gets parens; at the start it stays bare
        let e = Expr::bin(ADD, l("6"), Expr::bin(MOD, Expr::un(NEG, l("17")), l("7")));
        assert_eq!(e.render(), "6 + (-17) % 7");
        assert_eq!(Expr::bin(MUL, Expr::un(NEG, l("2")), l("5")).render(), "-2 * 5");
        assert_eq!(Expr::bin(POW, l("2"), Expr::un(NEG, l("3"))).render(), "2^(-3)");
        assert_eq!(Expr::un(SQRT, Expr::un(NEG, l("3"))).render(), "sqrt(-3)");
        // macros expand
        assert_eq!(Expr::un(MACRO0, l("3")).render(), "sec(atan(3))");
        assert_eq!(Expr::bin(COMB, l("5"), l("2")).render(), "combinations(5, 2)");
    }

    #[test]
    fn exact_verification() {
        // sec(atan(7-6)) * sec(atan(sqrt(7*7))) = sqrt(2) * sqrt(50) = 10
        let e = Expr::bin(
            MUL,
            Expr::un(MACRO0, Expr::bin(SUB, l("7"), l("6"))),
            Expr::un(MACRO0, Expr::un(SQRT, Expr::bin(MUL, l("7"), l("7")))),
        );
        assert!((e.eval().unwrap() - 10.0).abs() < 1e-12);
        assert!(e.verify_exact("10"));
        // (acos(0) + acos(-0!) + acos(-0!)) / atan(0!) = 10
        let neg1 = || Expr::un(ACOS, Expr::un(NEG, Expr::un(FACT, l("0"))));
        let e = Expr::bin(
            DIV,
            Expr::bin(ADD, Expr::bin(ADD, Expr::un(ACOS, l("0")), neg1()), neg1()),
            Expr::un(ATAN, Expr::un(FACT, l("0"))),
        );
        assert!(e.verify_exact("10"));
        // float underflow: 7^9^(-7!) + 9 is 10 in floats but not in math
        let neg_fact = Expr::un(NEG, Expr::un(FACT, l("7")));
        let e = Expr::bin(ADD, Expr::bin(POW, l("7"), Expr::bin(POW, l("9"), neg_fact)), l("9"));
        assert_eq!(e.eval(), Some(10.0));
        assert!(!e.verify_exact("10"));
        // a legit power with a negative exponent still verifies: 2^(-1) * 20
        let e = Expr::bin(MUL, Expr::bin(POW, l("2"), Expr::un(NEG, l("1"))), l("20"));
        assert!(e.verify_exact("10"));
        // 9.999999 is not 10
        let e = Expr::bin(DIV, l("9999999"), l("1000000"));
        assert!(!e.verify_exact("10"));
        // decimal targets are compared exactly: 1/10 == 0.1
        assert!(Expr::bin(DIV, l("1"), l("10")).verify_exact("0.1"));
        assert!(!Expr::bin(DIV, l("1"), l("10")).verify_exact("0.10000001"));
    }
}
