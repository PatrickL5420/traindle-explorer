//! Value quantisation for deduplication and lookups.
//!
//! Relative quantisation: the f64 mantissa is rounded to 40 bits, so a bucket is about
//! 1e-12 wide relative to the value. Because IEEE doubles are sign-magnitude, `key ± 1`
//! are the magnitude neighbours (also across exponent boundaries), which root lookups
//! probe to tolerate rounding noise. Correctness never depends on keys: every match is
//! re-evaluated numerically.

#[inline]
pub fn key(v: f64) -> u64 {
    if v == 0.0 {
        return 0; // +0 and -0 share a key; stored values keep their sign
    }
    v.to_bits().wrapping_add(1 << 11) >> 12
}

/// Raw keys of integers have all-zero low bits, so they must be mixed before hashing.
#[inline]
pub fn hkey(k: u64) -> u64 {
    let mut x = k.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_shares_or_neighbours_key() {
        let a = 1.0f64;
        let b = 0.999_999_999_999_999_9f64; // tan(atan(1)) in V8
        assert!(key(a).abs_diff(key(b)) <= 1);
        let c = 2.000_000_000_000_000_4f64; // sqrt(2)^2
        assert!(key(2.0).abs_diff(key(c)) <= 1);
        assert_ne!(key(10.0), key(10.001));
        assert_eq!(key(0.0), key(-0.0));
        assert_ne!(key(3.0), key(-3.0));
    }
}
