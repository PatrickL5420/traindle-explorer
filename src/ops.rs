//! Operator codes and numeric semantics.
//!
//! The arithmetic is real-number mathematics. An operation that is undefined over the
//! reals returns None: division by zero, `x % 0`, `0^0`, `0` to a negative power, a
//! negative base with a non-integer exponent, sqrt of a negative, factorial of a
//! negative, inverse trig outside its domain.
//!
//! The game evaluates expressions with mathjs, which is not the source of truth here. It
//! fixes the notation (precedence, function names) and, for a solution to be playable,
//! the game must compute the same value. Where mathjs bends the maths (`x % 0 = x`,
//! `0^0 = 1`, `acot(-0) = -pi/2`, a noise-tolerant floor), we reject the case instead of
//! copying it.
//!
//! Two layers:
//! * `raw_*`: f64 evaluation, as the game computes it, restricted to cases that are
//!   mathematically defined (None otherwise, or when the result is not finite). Used to
//!   re-evaluate final expression trees next to the 256-bit check in `expr.rs`.
//! * `bin` / `un`: the search versions. Same arithmetic, plus value filters (magnitude,
//!   fake zeros) and the EXACT / INT flags that track whether an f64 value is exactly
//!   the true value (so the game computes it identically).

use std::f64::consts::PI;

pub const LIT: u8 = 0;
// binary
pub const ADD: u8 = 1;
pub const SUB: u8 = 2;
pub const MUL: u8 = 3;
pub const DIV: u8 = 4;
pub const POW: u8 = 5;
pub const MOD: u8 = 6;
pub const COMB: u8 = 7;
pub const PERM: u8 = 8;
// unary
pub const NEG: u8 = 9;
pub const FACT: u8 = 10;
pub const SQRT: u8 = 11;
pub const CBRT: u8 = 12;
// forward trig
pub const SIN: u8 = 16;
pub const COS: u8 = 17;
pub const TAN: u8 = 18;
pub const SEC: u8 = 19;
pub const CSC: u8 = 20;
pub const COT: u8 = 21;
// inverse trig
pub const ASIN: u8 = 22;
pub const ACOS: u8 = 23;
pub const ATAN: u8 = 24;
pub const ASEC: u8 = 25;
pub const ACSC: u8 = 26;
pub const ACOT: u8 = 27;
// fused macros outer(inner(x)), one per distinct map on r = x^2
pub const MACRO0: u8 = 32;
pub const MACROS: [(u8, u8); 13] = [
    (SEC, ATAN), // r -> r + 1
    (COT, ATAN), // r -> 1 / r        (the legal reciprocal)
    (TAN, ASEC), // r -> r - 1
    (SIN, ATAN), // r -> r / (1 + r)
    (COS, ATAN), // r -> 1 / (1 + r)
    (CSC, ATAN), // r -> (1 + r) / r
    (SIN, ACOS), // r -> 1 - r
    (TAN, ACOS), // r -> (1 - r) / r
    (TAN, ASIN), // r -> r / (1 - r)
    (SEC, ASIN), // r -> 1 / (1 - r)
    (SIN, ASEC), // r -> (r - 1) / r
    (COT, ASEC), // r -> 1 / (r - 1)
    (CSC, ASEC), // r -> r / (r - 1)
];
pub const N_MACROS: u8 = MACROS.len() as u8;

pub const FWD_TRIG: [u8; 6] = [SIN, COS, TAN, SEC, CSC, COT];
pub const INV_TRIG: [u8; 6] = [ASIN, ACOS, ATAN, ASEC, ACSC, ACOT];

// value flags
pub const EXACT: u8 = 1; // the f64 is exactly the true value (no rounding so far)
pub const INT: u8 = 2; // value is an integer (in f64)

#[inline]
pub fn is_binary(op: u8) -> bool {
    (ADD..=PERM).contains(&op)
}
#[inline]
pub fn is_unary(op: u8) -> bool {
    op >= NEG
}
#[inline]
pub fn is_fwd_trig(op: u8) -> bool {
    (SIN..=COT).contains(&op)
}
#[inline]
pub fn is_inv_trig(op: u8) -> bool {
    (ASIN..=ACOT).contains(&op)
}
#[inline]
pub fn is_macro(op: u8) -> bool {
    (MACRO0..MACRO0 + N_MACROS).contains(&op)
}
#[inline]
pub fn macro_parts(op: u8) -> (u8, u8) {
    MACROS[(op - MACRO0) as usize]
}

pub fn name(op: u8) -> &'static str {
    match op {
        ADD => "+",
        SUB => "-",
        MUL => "*",
        DIV => "/",
        POW => "^",
        MOD => "%",
        COMB => "combinations",
        PERM => "permutations",
        NEG => "-",
        FACT => "!",
        SQRT => "sqrt",
        CBRT => "cbrt",
        SIN => "sin",
        COS => "cos",
        TAN => "tan",
        SEC => "sec",
        CSC => "csc",
        COT => "cot",
        ASIN => "asin",
        ACOS => "acos",
        ATAN => "atan",
        ASEC => "asec",
        ACSC => "acsc",
        ACOT => "acot",
        _ => "?",
    }
}

/// Precedence class in the game's notation (mathjs parser): higher binds tighter.
#[inline]
pub fn prec(op: u8) -> u8 {
    match op {
        ADD | SUB => 1,
        MUL | DIV | MOD => 2,
        NEG => 3,
        POW => 4,
        FACT => 5,
        _ => 6, // literal, function call
    }
}

/// May these digits be written as one literal? Multi-digit literals need the
/// concatenation feature and must not start with 0 ("06" is just 0 + 6 in disguise).
#[inline]
pub fn literal_allowed(digits: &[u8], concat: bool) -> bool {
    digits.len() == 1 || (concat && digits[0] != b'0')
}

// ---------------------------------------------------------------- raw semantics

/// JS `Number.isInteger`-style check used by mathjs `isInteger`.
#[inline]
pub fn js_is_int(x: f64) -> bool {
    x.is_finite() && x == x.round()
}

/// mathjs `nearlyEqual(a, b, relTol = 1e-12, absTol = 1e-15)`.
#[inline]
fn nearly_equal(a: f64, b: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return false;
    }
    if !a.is_finite() || !b.is_finite() {
        return a == b;
    }
    if a == b {
        return true;
    }
    (a - b).abs() <= (1e-12 * a.abs().max(b.abs())).max(1e-15)
}

/// mathjs `floor` for numbers (tolerant to float noise).
#[inline]
pub fn floor_js(x: f64) -> f64 {
    let f = x.floor();
    let r = x.round();
    if f == r {
        return f;
    }
    if nearly_equal(x, r) && !nearly_equal(x, f) {
        return r;
    }
    f
}

/// Floored modulo `x - y * floor(x / y)` (the result takes the divisor's sign).
/// `x % 0` is undefined (mathjs returns x; we don't). The floor is mathjs's
/// noise-tolerant one so the f64 value matches the game; where it differs from the
/// exact floor, the 256-bit check in `expr.rs` rejects the expression.
#[inline]
pub fn mod_js(x: f64, y: f64) -> Option<f64> {
    if y == 0.0 {
        None
    } else {
        Some(x - y * floor_js(x / y))
    }
}

/// Real power. None when undefined over the reals: `0^0`, `0` to a negative power
/// (non-finite), or a negative base with a non-integer exponent (complex).
#[inline]
pub fn pow_js(x: f64, y: f64) -> Option<f64> {
    if x == 0.0 && y == 0.0 {
        None
    } else if js_is_int(y) || x >= 0.0 {
        Some(libm::pow(x, y))
    } else {
        None
    }
}

/// mathjs `utils/product`: divide-and-conquer product of i..=n.
fn product(i: i64, n: i64) -> f64 {
    if n < i {
        return 1.0;
    }
    if n == i {
        return n as f64;
    }
    let half = (n + i) >> 1;
    product(i, half) * product(half + 1, n)
}

const GAMMA_G: f64 = 4.7421875;
#[allow(clippy::excessive_precision)]
const GAMMA_P: [f64; 15] = [
    0.99999999999999709182,
    57.156235665862923517,
    -59.597960355475491248,
    14.136097974741747174,
    -0.49191381609762019978,
    0.33994649984811888699e-4,
    0.46523628927048575665e-4,
    -0.98374475304879564677e-4,
    0.15808870322491248884e-3,
    -0.21026444172410488319e-3,
    0.21743961811521264320e-3,
    -0.16431810653676389022e-3,
    0.84418223983852743293e-4,
    -0.26190838401581408670e-4,
    0.36899182659531622704e-5,
];

/// Port of mathjs `gammaNumber`.
pub fn gamma_js(n: f64) -> f64 {
    if js_is_int(n) {
        if n <= 0.0 {
            return f64::INFINITY;
        }
        if n > 171.0 {
            return f64::INFINITY;
        }
        return product(1, n as i64 - 1);
    }
    if n < 0.5 {
        return PI / (libm::sin(PI * n) * gamma_js(1.0 - n));
    }
    if n >= 171.35 {
        return f64::INFINITY;
    }
    if n > 85.0 {
        let two_n = n * n;
        let three_n = two_n * n;
        let four_n = three_n * n;
        let five_n = four_n * n;
        return (2.0 * PI / n).sqrt()
            * libm::pow(n / std::f64::consts::E, n)
            * (1.0 + 1.0 / (12.0 * n) + 1.0 / (288.0 * two_n)
                - 139.0 / (51840.0 * three_n)
                - 571.0 / (2488320.0 * four_n)
                + 163879.0 / (209018880.0 * five_n)
                + 5246819.0 / (75246796800.0 * five_n * n));
    }
    let n = n - 1.0;
    let mut x = GAMMA_P[0];
    for (i, p) in GAMMA_P.iter().enumerate().skip(1) {
        x += p / (n + i as f64);
    }
    let t = n + GAMMA_G + 0.5;
    (2.0 * PI).sqrt() * libm::pow(t, n + 0.5) * libm::exp(-t) * x
}

/// Exact integer combinations / permutations (mathjs semantics), None when invalid
/// or when the result leaves the exact-integer range.
pub fn comb_perm(op: u8, n: f64, k: f64) -> Option<f64> {
    if !js_is_int(n) || n < 0.0 || !js_is_int(k) || k < 0.0 || k > n {
        return None;
    }
    if n > 1e15 {
        return None;
    }
    let (n, k) = (n as u128, k as u128);
    const LIMIT: u128 = 1 << 53;
    if op == COMB {
        let k = k.min(n - k);
        let mut r: u128 = 1;
        for i in 1..=k {
            r = r * (n - k + i) / i;
            if r >= LIMIT {
                return None;
            }
        }
        Some(r as f64)
    } else {
        let mut r: u128 = 1;
        for i in 0..k {
            r *= n - i;
            if r >= LIMIT {
                return None;
            }
        }
        Some(r as f64)
    }
}

#[inline]
fn fin(x: f64) -> Option<f64> {
    if x.is_finite() {
        Some(x)
    } else {
        None
    }
}

/// f64 value of a binary operator (None = undefined or not finite).
pub fn raw_bin(op: u8, a: f64, b: f64) -> Option<f64> {
    match op {
        ADD => fin(a + b),
        SUB => fin(a - b),
        MUL => fin(a * b),
        DIV => fin(a / b),
        POW => fin(pow_js(a, b)?),
        MOD => fin(mod_js(a, b)?),
        COMB | PERM => comb_perm(op, a, b),
        _ => None,
    }
}

/// f64 value of a unary operator / function (None = undefined or not finite).
pub fn raw_un(op: u8, x: f64) -> Option<f64> {
    let r = match op {
        NEG => -x,
        FACT => {
            if x < 0.0 {
                return None;
            }
            gamma_js(x + 1.0)
        }
        SQRT => {
            if x >= 0.0 {
                x.sqrt()
            } else {
                return None;
            }
        }
        CBRT => libm::cbrt(x),
        SIN => libm::sin(x),
        COS => libm::cos(x),
        TAN => libm::tan(x),
        SEC => 1.0 / libm::cos(x),
        CSC => 1.0 / libm::sin(x),
        COT => 1.0 / libm::tan(x),
        ASIN => {
            if (-1.0..=1.0).contains(&x) {
                libm::asin(x)
            } else {
                return None;
            }
        }
        ACOS => {
            if (-1.0..=1.0).contains(&x) {
                libm::acos(x)
            } else {
                return None;
            }
        }
        ATAN => libm::atan(x),
        ASEC => {
            if x <= -1.0 || x >= 1.0 {
                libm::acos(1.0 / x)
            } else {
                return None;
            }
        }
        ACSC => {
            if x <= -1.0 || x >= 1.0 {
                libm::asin(1.0 / x)
            } else {
                return None;
            }
        }
        ACOT => {
            // acot(0) = pi/2. A negative zero (e.g. from "-0") would make the game
            // compute atan(-inf) = -pi/2, which is wrong, so it is rejected.
            if x == 0.0 && x.is_sign_negative() {
                return None;
            }
            libm::atan(1.0 / x)
        }
        _ if is_macro(op) => {
            let (outer, inner) = macro_parts(op);
            return raw_un(outer, raw_un(inner, x)?);
        }
        _ => return None,
    };
    fin(r)
}

// ---------------------------------------------------------------- search semantics

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_abs: f64,
    pub min_abs: f64,
    /// largest n with n! <= max_abs
    pub fact_max: f64,
    /// allow factorial of non-integers (gamma)
    pub gamma: bool,
}

impl Limits {
    pub fn new(max_abs: f64, gamma: bool) -> Self {
        let mut n = 0.0;
        let mut f = 1.0;
        while f * (n + 1.0) <= max_abs {
            n += 1.0;
            f *= n;
        }
        Limits { max_abs, min_abs: 1e-9, fact_max: n, gamma }
    }
}

#[inline]
fn finish(r: f64, exact: bool, lim: &Limits) -> Option<(f64, u8)> {
    if !r.is_finite() {
        return None;
    }
    let a = r.abs();
    if a > lim.max_abs || (r != 0.0 && a < lim.min_abs) {
        return None;
    }
    let mut f = 0;
    if exact {
        f |= EXACT;
    }
    if r == r.round() {
        f |= INT;
    }
    // keep the sign of zero: `raw_un` rejects acot(-0), which the game gets wrong
    Some((r, f))
}

#[inline]
pub fn bin_add(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    let r = a + b;
    if r != 0.0 && r.abs() <= 1e-12 * a.abs().max(b.abs()) {
        return None; // cancellation noise
    }
    finish(r, fa & fb & EXACT != 0, lim)
}
#[inline]
pub fn bin_sub(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    let r = a - b;
    if r != 0.0 && r.abs() <= 1e-12 * a.abs().max(b.abs()) {
        return None;
    }
    finish(r, fa & fb & EXACT != 0, lim)
}
#[inline]
pub fn bin_mul(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    let r = a * b;
    if r == 0.0 && a != 0.0 && b != 0.0 {
        return None; // underflow: a product of nonzero numbers is never 0
    }
    finish(r, fa & fb & EXACT != 0, lim)
}
#[inline]
pub fn bin_div(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    if b == 0.0 {
        return None;
    }
    let r = a / b;
    if r == 0.0 && a != 0.0 {
        return None; // underflow
    }
    finish(r, fa & fb & EXACT != 0, lim)
}
#[inline]
pub fn bin_pow(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    if a < 0.0 && (fb & EXACT == 0 || fb & INT == 0) {
        return None; // not real (or ambiguous integer-ness)
    }
    if a == 0.0 && b <= 0.0 {
        return None; // 0^0 is undefined, 0^-n is division by zero
    }
    if b.abs() > 1e4 {
        return None;
    }
    // |a^b| = e^(b ln|a|): skip powers that would land outside [min_abs, max_abs] before
    // computing them. In particular a huge negative exponent (9^(-7!) = 4e-4810) would
    // underflow to exactly 0, and "7^9^(-7!) = 7^0 = 1" is a float artefact, not math.
    if a != 0.0 && a.abs() != 1.0 {
        let e = b * a.abs().ln();
        if e < lim.min_abs.ln() - 1.0 || e > lim.max_abs.ln() + 1.0 {
            return None;
        }
    }
    let r = libm::pow(a, b);
    if r == 0.0 && a != 0.0 {
        return None; // underflow: a^b is never 0 for a != 0
    }
    let exact = fa & fb & EXACT != 0 && fa & INT != 0 && fb & INT != 0 && b >= 0.0 && r.abs() < 9.0e15;
    finish(r, exact, lim)
}
#[inline]
pub fn bin_mod(a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    // x - y*floor(x/y) multiplies any ulp error in y by the quotient: with an inexact
    // operand and a large quotient, Rust and V8 could disagree beyond our tolerance
    if fa & fb & EXACT == 0 && b != 0.0 && (a / b).abs() > 1e4 {
        return None;
    }
    let r = mod_js(a, b)?;
    if r != 0.0 && r.abs() <= 1e-12 * a.abs() {
        return None;
    }
    // a non-exact "zero remainder" is usually float noise
    if r == 0.0 && fa & fb & EXACT == 0 {
        return None;
    }
    finish(r, fa & fb & EXACT != 0, lim)
}
#[inline]
pub fn bin_comb_perm(op: u8, a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    // defined for integers 0 <= k <= n only, so only trust bit-exact integer operands
    if fa & fb & EXACT == 0 || fa & fb & INT == 0 {
        return None;
    }
    finish(comb_perm(op, a, b)?, true, lim)
}

pub fn bin(op: u8, a: f64, fa: u8, b: f64, fb: u8, lim: &Limits) -> Option<(f64, u8)> {
    match op {
        ADD => bin_add(a, fa, b, fb, lim),
        SUB => bin_sub(a, fa, b, fb, lim),
        MUL => bin_mul(a, fa, b, fb, lim),
        DIV => bin_div(a, fa, b, fb, lim),
        POW => bin_pow(a, fa, b, fb, lim),
        MOD => bin_mod(a, fa, b, fb, lim),
        COMB | PERM => bin_comb_perm(op, a, fa, b, fb, lim),
        _ => None,
    }
}

/// Factorial of small non-negative integers (exact for n <= 18).
#[inline]
pub fn fact_int(n: u32) -> f64 {
    (2..=n).fold(1.0, |a, i| a * i as f64)
}

/// Is `v` a rational multiple of pi with denominator dividing 120 (|v| <= 2pi)?
#[inline]
pub fn is_pirat(v: f64) -> bool {
    if v.abs() > 2.0 * PI + 1e-9 {
        return false;
    }
    let k = v * (120.0 / PI);
    (k - k.round()).abs() < 1e-9
}

#[inline]
fn domain_ambiguous(op: u8, x: f64, exact: bool) -> bool {
    // A non-exact value sitting on a domain edge (|x| = 1) may be just inside in Rust
    // and just outside in V8 (or vice versa): don't risk it.
    !exact && matches!(op, ASIN | ACOS | ASEC | ACSC) && (x.abs() - 1.0).abs() < 1e-12
}

fn trig_step(op: u8, x: f64, exact: bool) -> Option<f64> {
    if domain_ambiguous(op, x, exact) {
        return None;
    }
    let r = raw_un(op, x)?;
    if r != 0.0 && r.abs() < 1e-12 {
        return None; // e.g. sin(pi) = 1.2e-16
    }
    if r.abs() > 1e12 {
        return None; // e.g. tan(pi/2)
    }
    Some(r)
}

/// Search-time unary op. Callers handle "useless application" pre-filters.
pub fn un(op: u8, x: f64, fx: u8, lim: &Limits) -> Option<(f64, u8)> {
    let ex = fx & EXACT != 0;
    match op {
        NEG => finish(-x, ex, lim),
        FACT => {
            if x < 0.0 {
                None
            } else if fx & INT != 0 {
                if x > lim.fact_max {
                    None
                } else {
                    finish(fact_int(x as u32), ex, lim)
                }
            } else if lim.gamma {
                finish(gamma_js(x + 1.0), false, lim)
            } else {
                None
            }
        }
        SQRT => {
            if x < 0.0 {
                None
            } else {
                finish(x.sqrt(), ex, lim)
            }
        }
        CBRT => {
            let r = libm::cbrt(x);
            let exact = ex && r == r.round() && r * r * r == x;
            finish(r, exact, lim)
        }
        _ if is_fwd_trig(op) || is_inv_trig(op) => {
            let r = trig_step(op, x, ex)?;
            let exact = ex && (x == 0.0 || x.abs() == 1.0) && (r == 0.0 || r.abs() == 1.0);
            finish(r, exact, lim)
        }
        _ if is_macro(op) => {
            let (outer, inner) = macro_parts(op);
            let mid = trig_step(inner, x, ex)?;
            let r = trig_step(outer, mid, false)?;
            finish(r, false, lim)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floored_mod() {
        assert_eq!(mod_js(7.0, 3.0), Some(1.0));
        assert_eq!(mod_js(-7.0, 3.0), Some(2.0));
        assert_eq!(mod_js(7.0, -3.0), Some(-2.0));
        assert_eq!(mod_js(5.5, 2.0), Some(1.5));
        assert_eq!(mod_js(100.0, 0.0), None); // undefined (mathjs says 100)
        assert_eq!(mod_js(720.0, 71.0), Some(10.0));
        assert_eq!(mod_js(17.0, -7.0), Some(-4.0));
    }

    #[test]
    fn gamma_matches_mathjs() {
        assert_eq!(raw_un(FACT, 0.5).unwrap(), 0.886_226_925_452_757_6);
        assert_eq!(raw_un(FACT, 3.0).unwrap(), 6.0);
        assert_eq!(raw_un(FACT, 0.0).unwrap(), 1.0);
        assert!(raw_un(FACT, -1.0).is_none());
    }

    #[test]
    fn pow_and_roots() {
        assert_eq!(raw_bin(POW, 0.0, 0.0), None); // undefined (mathjs says 1)
        assert_eq!(raw_bin(POW, 0.0, 2.0), Some(0.0));
        assert!(raw_bin(POW, 0.0, -1.0).is_none());
        assert_eq!(raw_bin(POW, 2.0, 10.0), Some(1024.0));
        assert!(raw_bin(POW, -8.0, 1.0 / 3.0).is_none());
        assert_eq!(raw_bin(POW, -2.0, 3.0), Some(-8.0));
        assert_eq!(raw_un(CBRT, -8.0), Some(-2.0));
        assert_eq!(raw_un(CBRT, 8.0), Some(2.0));
        assert!(raw_un(SQRT, -4.0).is_none());
    }

    #[test]
    fn comb_perm_semantics() {
        assert_eq!(comb_perm(COMB, 5.0, 2.0), Some(10.0));
        assert_eq!(comb_perm(COMB, 0.0, 0.0), Some(1.0));
        assert_eq!(comb_perm(PERM, 5.0, 2.0), Some(20.0));
        assert_eq!(comb_perm(PERM, 3.0, 0.0), Some(1.0));
        assert!(comb_perm(COMB, 2.0, 5.0).is_none());
        assert!(comb_perm(COMB, 5.5, 2.0).is_none());
    }

    #[test]
    fn trig_semantics() {
        assert_eq!(raw_un(ACOS, -1.0), Some(PI));
        assert!(raw_un(ASEC, 0.0).is_none());
        assert!(raw_un(ASIN, 1.5).is_none());
        assert_eq!(raw_un(ACOT, 0.0), Some(PI / 2.0));
        assert!(raw_un(ACOT, -0.0).is_none()); // the game would say -pi/2
        let m = MACRO0; // sec(atan(3)) = sqrt(10)
        assert!((raw_un(m, 3.0).unwrap() - 10f64.sqrt()).abs() < 1e-15);
        assert!(is_pirat(PI / 4.0));
        assert!(is_pirat(0.0));
        assert!(!is_pirat(3.0));
    }

    #[test]
    fn concat_rules() {
        assert!(literal_allowed(b"7", false));
        assert!(!literal_allowed(b"12", false));
        assert!(literal_allowed(b"12", true));
        assert!(!literal_allowed(b"06", true));
        assert!(!literal_allowed(b"0010", true));
        assert!(literal_allowed(b"0", false));
    }

    #[test]
    fn no_underflow_zeros() {
        let lim = Limits::new(1e7, false);
        let ex = EXACT | INT;
        // 9^(-7!) is 4e-4810, not 0
        assert!(bin_pow(9.0, ex, -5040.0, ex, &lim).is_none());
        assert!(bin_pow(9.0, ex, -720.0, ex, &lim).is_none());
        // genuine zeros and ordinary powers still work
        assert_eq!(bin_pow(0.0, ex, 3.0, ex, &lim).unwrap().0, 0.0);
        assert_eq!(bin_pow(2.0, ex, -3.0, ex, &lim).unwrap().0, 0.125);
        assert_eq!(bin_pow(1.0, ex, -5040.0, ex, &lim).unwrap().0, 1.0);
        assert_eq!(bin_mul(0.0, ex, 7.0, ex, &lim).unwrap().0, 0.0);
        assert!(bin_mul(1e-200, 0, 1e-200, 0, &Limits { min_abs: 0.0, ..lim }).is_none());
    }

    #[test]
    fn factorial_limits() {
        let lim = Limits::new(1e7, false);
        assert_eq!(lim.fact_max, 10.0);
        assert_eq!(un(FACT, 10.0, EXACT | INT, &lim).unwrap().0, 3628800.0);
        assert!(un(FACT, 11.0, EXACT | INT, &lim).is_none());
    }
}
