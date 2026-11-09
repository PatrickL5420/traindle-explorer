//! Solver configuration: features grouped into ordered tiers, cost weights, digit
//! count, target, plus the round schedule (search budgets) derived from them.

use crate::ops::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A unit of the tier editor. Each operator/function family is one feature.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Feature {
    Add,
    Sub,
    Mul,
    Div,
    Brackets,
    Neg,
    Pow,
    Concat,
    Fact,
    Mod,
    Sqrt,
    Cbrt,
    Comb,
    Perm,
    Trig,   // sin cos tan
    Atrig,  // asin acos atan
    Rtrig,  // sec csc cot
    Artrig, // asec acsc acot
}

pub const ALL_FEATURES: [Feature; 18] = [
    Feature::Add,
    Feature::Sub,
    Feature::Mul,
    Feature::Div,
    Feature::Brackets,
    Feature::Neg,
    Feature::Pow,
    Feature::Concat,
    Feature::Fact,
    Feature::Mod,
    Feature::Sqrt,
    Feature::Cbrt,
    Feature::Comb,
    Feature::Perm,
    Feature::Trig,
    Feature::Atrig,
    Feature::Rtrig,
    Feature::Artrig,
];

impl Feature {
    #[inline]
    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }
    pub fn label(self) -> &'static str {
        match self {
            Feature::Add => "+",
            Feature::Sub => "-",
            Feature::Mul => "*",
            Feature::Div => "/",
            Feature::Brackets => "( )",
            Feature::Neg => "unary -",
            Feature::Pow => "^",
            Feature::Concat => "concat",
            Feature::Fact => "!",
            Feature::Mod => "%",
            Feature::Sqrt => "sqrt",
            Feature::Cbrt => "cbrt",
            Feature::Comb => "combinations",
            Feature::Perm => "permutations",
            Feature::Trig => "sin cos tan",
            Feature::Atrig => "asin acos atan",
            Feature::Rtrig => "sec csc cot",
            Feature::Artrig => "asec acsc acot",
        }
    }
    /// Function-style features are written with parentheses, so they cannot appear in
    /// a tier before brackets are allowed.
    pub fn needs_brackets(self) -> bool {
        matches!(
            self,
            Feature::Fact
                | Feature::Sqrt
                | Feature::Cbrt
                | Feature::Comb
                | Feature::Perm
                | Feature::Trig
                | Feature::Atrig
                | Feature::Rtrig
                | Feature::Artrig
        )
    }
    /// The feature an operator code belongs to (literals: see Concat).
    pub fn of_op(op: u8) -> Option<Feature> {
        Some(match op {
            ADD => Feature::Add,
            SUB => Feature::Sub,
            MUL => Feature::Mul,
            DIV => Feature::Div,
            NEG => Feature::Neg,
            POW => Feature::Pow,
            FACT => Feature::Fact,
            MOD => Feature::Mod,
            SQRT => Feature::Sqrt,
            CBRT => Feature::Cbrt,
            COMB => Feature::Comb,
            PERM => Feature::Perm,
            SIN | COS | TAN => Feature::Trig,
            ASIN | ACOS | ATAN => Feature::Atrig,
            SEC | CSC | COT => Feature::Rtrig,
            ASEC | ACSC | ACOT => Feature::Artrig,
            _ => return None,
        })
    }
    /// Key used in the weights map.
    pub fn weight_key(self) -> Option<&'static str> {
        Some(match self {
            Feature::Add => "add",
            Feature::Sub => "sub",
            Feature::Mul => "mul",
            Feature::Div => "div",
            Feature::Neg => "neg",
            Feature::Pow => "pow",
            Feature::Concat => "concat",
            Feature::Fact => "fact",
            Feature::Mod => "mod",
            Feature::Sqrt => "sqrt",
            Feature::Cbrt => "cbrt",
            Feature::Comb => "comb",
            Feature::Perm => "perm",
            Feature::Trig | Feature::Atrig | Feature::Rtrig | Feature::Artrig => "trig",
            Feature::Brackets => return None,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TierDef {
    #[serde(default)]
    pub name: String,
    pub features: Vec<Feature>,
}

/// Everything that defines a run. This is what the web page edits and what is saved
/// with each run.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SolverConfig {
    pub name: String,
    /// number of digits per puzzle (1..=5)
    pub digits: usize,
    /// target value, as a decimal string (e.g. "10", "-3", "0.5")
    pub target: String,
    pub tiers: Vec<TierDef>,
    /// cost weights by key (add sub mul div neg pow concat fact mod sqrt cbrt comb perm trig)
    pub weights: BTreeMap<String, u32>,
    // ---- advanced (CLI only for now)
    pub profile: Profile,
    pub max_abs: f64,
    pub gamma: bool,
    /// trig search depth: how many trig rounds to run (1-3). The 3rd (unfiltered
    /// inverse trig, biggest budget) is a slow last resort.
    pub trig_rounds: u8,
    /// rarity window: also collect verified solutions costing at most best + this
    /// (0 = keep only the best, fastest)
    pub alt_slack: u32,
}

pub fn default_weights() -> BTreeMap<String, u32> {
    [
        ("add", 10),
        ("sub", 10),
        ("mul", 10),
        ("div", 11),
        ("neg", 6),
        ("pow", 12),
        ("concat", 4),
        ("fact", 12),
        ("mod", 13),
        ("sqrt", 14),
        ("cbrt", 15),
        ("comb", 16),
        ("perm", 16),
        ("trig", 18),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

pub fn default_tiers() -> Vec<TierDef> {
    use Feature::*;
    let t = |name: &str, f: &[Feature]| TierDef { name: name.into(), features: f.to_vec() };
    vec![
        t("+-*/", &[Add, Sub, Mul, Div, Neg]),
        t("brackets", &[Brackets]),
        t("^", &[Pow]),
        t("concat", &[Concat]),
        t("!", &[Fact]),
        t("%", &[Mod]),
        t("sqrt", &[Sqrt]),
        t("cbrt", &[Cbrt]),
        t("combinations", &[Comb]),
        t("permutations", &[Perm]),
        t("trig", &[Trig, Atrig, Rtrig, Artrig]),
    ]
}

impl Default for SolverConfig {
    fn default() -> Self {
        SolverConfig {
            name: String::new(),
            digits: 4,
            target: "10".into(),
            tiers: default_tiers(),
            weights: default_weights(),
            profile: Profile::Default,
            max_abs: 1e7,
            gamma: false,
            trig_rounds: 2,
            alt_slack: 10,
        }
    }
}

impl SolverConfig {
    /// Check the config and fill in derived fields (tier names, missing weights).
    pub fn normalize(&mut self) -> Result<(), String> {
        if !(1..=5).contains(&self.digits) {
            return Err("digits must be between 1 and 5".into());
        }
        self.target = self.target.trim().to_string();
        if self.target.contains(['e', 'E']) {
            return Err("write the target without exponent notation".into());
        }
        let t: f64 = self.target.parse().map_err(|_| format!("target {:?} is not a number", self.target))?;
        if !t.is_finite() || t.abs() > 1e6 {
            return Err("target must be a finite number with |target| <= 1e6".into());
        }
        self.tiers.retain(|t| !t.features.is_empty());
        if self.tiers.is_empty() {
            return Err("at least one tier needs features".into());
        }
        if self.tiers.len() > 30 {
            return Err("at most 30 tiers".into());
        }
        let mut seen = 0u32;
        let mut brackets_at: Option<usize> = None;
        for (i, tier) in self.tiers.iter().enumerate() {
            for &f in &tier.features {
                if seen & f.bit() != 0 {
                    return Err(format!("feature {} appears twice", f.label()));
                }
                seen |= f.bit();
                if f == Feature::Brackets {
                    brackets_at = Some(i);
                }
            }
        }
        for (i, tier) in self.tiers.iter().enumerate() {
            for &f in &tier.features {
                if f.needs_brackets() && brackets_at.is_none_or(|b| b > i) {
                    return Err(format!(
                        "{} is written with parentheses, so brackets must be in tier {} or earlier",
                        f.label(),
                        i + 1
                    ));
                }
            }
        }
        for tier in &mut self.tiers {
            tier.features.sort();
            tier.name = tier.name.trim().to_string();
            if tier.name.is_empty() {
                tier.name = tier.features.iter().map(|f| f.label()).collect::<Vec<_>>().join(" ");
            }
        }
        for (k, v) in default_weights() {
            self.weights.entry(k).or_insert(v);
        }
        for (k, &w) in &self.weights {
            if w > 1000 {
                return Err(format!("weight {k} is too large (max 1000)"));
            }
            if w == 0 && k != "concat" {
                return Err("operator weights must be at least 1 (only concat may be 0)".into());
            }
        }
        if !(1..=3).contains(&self.trig_rounds) {
            return Err("trig rounds must be 1, 2 or 3".into());
        }
        if self.alt_slack > 100 {
            return Err("alternatives window must be 0-100".into());
        }
        if self.max_abs <= 0.0 || !self.max_abs.is_finite() {
            self.max_abs = 1e7;
        }
        Ok(())
    }

    pub fn target_f64(&self) -> f64 {
        self.target.trim().parse().unwrap_or(10.0)
    }

    /// 1-based tier of each feature (0 = disabled).
    pub fn feature_tiers(&self) -> [u8; 18] {
        let mut out = [0u8; 18];
        for (i, tier) in self.tiers.iter().enumerate() {
            for &f in &tier.features {
                out[f as usize] = (i + 1) as u8;
            }
        }
        out
    }

    /// Tier of an expression given its feature bitmask (0 = uses a disabled feature).
    pub fn tier_of_mask(&self, mask: u32) -> u8 {
        let ft = self.feature_tiers();
        let mut t = 1u8;
        for f in ALL_FEATURES {
            if mask & f.bit() != 0 {
                if ft[f as usize] == 0 {
                    return 0;
                }
                t = t.max(ft[f as usize]);
            }
        }
        t
    }
}

/// Cost weights indexed by op code. Lower = simpler.
#[derive(Clone, Debug)]
pub struct Weights {
    ops: [u32; 64],
    pub concat: u32,
}

impl Weights {
    pub fn from_map(m: &BTreeMap<String, u32>) -> Self {
        let defaults = default_weights();
        let g = |k: &str| *m.get(k).unwrap_or(&defaults[k]);
        let mut ops = [18u32; 64];
        for op in 0..64u8 {
            if let Some(f) = Feature::of_op(op) {
                ops[op as usize] = g(f.weight_key().unwrap());
            }
        }
        for i in 0..N_MACROS {
            ops[(MACRO0 + i) as usize] = 2 * g("trig");
        }
        Weights { ops, concat: g("concat") }
    }
    #[inline]
    pub fn op(&self, op: u8) -> u32 {
        self.ops[op as usize]
    }
    pub fn literal(&self, digits: &[u8]) -> u32 {
        self.concat * (digits.len() as u32 - 1)
    }
}

/// One search round: the operators enabled up to a tier, plus a cost budget.
#[derive(Clone, Debug)]
pub struct Round {
    pub label: String,
    /// no brackets yet: solved by the flat (precedence-only) enumerator
    pub flat: bool,
    pub concat: bool,
    pub neg: bool,
    pub binops: Vec<u8>,
    pub unops: Vec<u8>,
    /// maximum total cost of a solution found in this round
    pub c_total: u32,
    /// cap on entries per substring value set (truncates by cost)
    pub max_entries: usize,
    /// cost budget for the unary chain applied on top of the root binary op
    pub root_budget: u32,
    /// keep inverse-trig results only when they are rational multiples of pi
    pub pi_filter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// Budgets tuned so the default 4-digit game is solved before the last trig round
    #[default]
    Default,
    /// Larger cost budgets and set caps: slower, may find cheaper deep solutions
    Thorough,
}

fn ops_for(mask: u32) -> (Vec<u8>, Vec<u8>) {
    let has = |f: Feature| mask & f.bit() != 0;
    let mut bin = vec![];
    for (f, op) in [
        (Feature::Add, ADD),
        (Feature::Sub, SUB),
        (Feature::Mul, MUL),
        (Feature::Div, DIV),
        (Feature::Pow, POW),
        (Feature::Mod, MOD),
        (Feature::Comb, COMB),
        (Feature::Perm, PERM),
    ] {
        if has(f) {
            bin.push(op);
        }
    }
    let mut un = vec![];
    for (f, op) in [(Feature::Neg, NEG), (Feature::Fact, FACT), (Feature::Sqrt, SQRT), (Feature::Cbrt, CBRT)] {
        if has(f) {
            un.push(op);
        }
    }
    let trig_on = |op: u8| Feature::of_op(op).is_some_and(has);
    for op in FWD_TRIG.into_iter().chain(INV_TRIG) {
        if trig_on(op) {
            un.push(op);
        }
    }
    for i in 0..N_MACROS {
        let (outer, inner) = MACROS[i as usize];
        if trig_on(outer) && trig_on(inner) {
            un.push(MACRO0 + i);
        }
    }
    (bin, un)
}

/// The round schedule. Each round only runs on numbers still unsolved.
pub fn rounds(cfg: &SolverConfig) -> Vec<Round> {
    let (scale_c, scale_e) = match cfg.profile {
        Profile::Default => (1.0, 1.0),
        Profile::Thorough => (1.2, 2.0),
    };
    let m = 1_000_000usize;
    let mut out = vec![];
    let mut mask = 0u32;
    for (i, tier) in cfg.tiers.iter().enumerate() {
        for f in &tier.features {
            mask |= f.bit();
        }
        let has = |f: Feature| mask & f.bit() != 0;
        let (binops, unops) = ops_for(mask);
        let base = Round {
            label: format!("T{} {}", i + 1, tier.name),
            flat: !has(Feature::Brackets),
            concat: has(Feature::Concat),
            neg: has(Feature::Neg),
            binops,
            unops,
            c_total: 100_000,
            max_entries: 8 * m,
            root_budget: 6,
            pi_filter: true,
        };
        if base.flat {
            out.push(base);
            continue;
        }
        let trig = has(Feature::Trig) || has(Feature::Atrig) || has(Feature::Rtrig) || has(Feature::Artrig);
        let level2 = has(Feature::Sqrt) || has(Feature::Cbrt) || has(Feature::Comb) || has(Feature::Perm);
        let level1 = has(Feature::Fact) || has(Feature::Mod);
        let root_budget = if trig {
            80
        } else if has(Feature::Sqrt) || has(Feature::Cbrt) {
            40
        } else {
            6
        };
        // (sub-label, c_total, max_entries, pi_filter). Without unary growth beyond neg
        // the sets stay small, so those tiers are searched exhaustively.
        let modx = if has(Feature::Mod) && !level2 && !trig { 10 } else { 0 };
        // Short puzzles: few unsolved numbers ever get solved by deep unary chains, but
        // their tiny substrings (both sides of a 1|1 split) explode in size, so the
        // deep budgets shrink with the digit count.
        let spec: Vec<(&str, u32, usize, bool)> = if trig {
            match cfg.digits {
                1 | 2 => vec![("a", 120, m / 2, true), ("b", 170, m, true)],
                3 => vec![("a", 120, 2 * m, true), ("b", 170, 3 * m, true), ("c", 220, 3 * m, false)],
                _ => vec![("a", 120, 4 * m, true), ("b", 170, 6 * m, true), ("c", 220, 6 * m, false)],
            }
        } else if level2 {
            match cfg.digits {
                1 | 2 => vec![("a", 120, m / 2, true), ("b", 170, m, true)],
                _ => vec![("a", 120, 3 * m, true), ("b", 170, 3 * m, true)],
            }
        } else if level1 {
            vec![("a", 100 + modx, 2 * m, true), ("b", 140 + modx, 2 * m, true)]
        } else {
            vec![("", 100_000, 8 * m, true)]
        };
        let mut spec = spec;
        if trig {
            spec.truncate(cfg.trig_rounds as usize);
        }
        let exhaustive = spec.len() == 1 && !trig;
        for (sub, c, e, pi) in spec {
            let mut r = base.clone();
            r.label = format!("T{}{} {}", i + 1, sub, tier.name);
            r.c_total = if exhaustive { c } else { (c as f64 * scale_c) as u32 };
            r.max_entries = if exhaustive { e } else { (e as f64 * scale_e) as usize };
            r.root_budget = root_budget;
            r.pi_filter = pi;
            out.push(r);
        }
    }
    out
}

/// Runtime config derived from a SolverConfig.
#[derive(Clone, Debug)]
pub struct Config {
    pub w: Weights,
    pub lim: Limits,
    pub target: f64,
    /// exact decimal target, for the 256-bit check
    pub target_str: String,
    /// |value - target| tolerance for the f64 check (the 256-bit check is far stricter)
    pub tol: f64,
    /// see `SolverConfig::alt_slack`
    pub slack: u32,
}

impl Config {
    pub fn from(sc: &SolverConfig) -> Self {
        let target = sc.target_f64();
        Config {
            w: Weights::from_map(&sc.weights),
            lim: Limits::new(sc.max_abs, sc.gamma),
            target,
            target_str: sc.target.clone(),
            tol: 1e-10 * target.abs().max(1.0),
            slack: sc.alt_slack,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_ordered() {
        let mut c = SolverConfig::default();
        c.normalize().unwrap();
        let ft = c.feature_tiers();
        assert_eq!(ft[Feature::Add as usize], 1);
        assert_eq!(ft[Feature::Concat as usize], 4);
        assert_eq!(ft[Feature::Artrig as usize], 11);
        let r = rounds(&c);
        assert!(r[0].flat && !r[0].concat && r[0].neg);
        assert!(!r[1].flat);
    }

    #[test]
    fn function_before_brackets_is_rejected() {
        let mut c = SolverConfig::default();
        c.tiers[0].features.push(Feature::Sqrt);
        c.tiers[6].features.clear();
        assert!(c.normalize().is_err());
    }

    #[test]
    fn trig_rounds_limit_the_schedule() {
        let count = |n: u8| {
            let mut c = SolverConfig { trig_rounds: n, ..SolverConfig::default() };
            c.normalize().unwrap();
            rounds(&c).iter().filter(|r| r.label.starts_with("T11")).count()
        };
        assert_eq!((count(1), count(2), count(3)), (1, 2, 3));
        let mut bad = SolverConfig { trig_rounds: 4, ..SolverConfig::default() };
        assert!(bad.normalize().is_err());
    }

    #[test]
    fn macros_need_both_trig_groups() {
        let (_, un) = ops_for(Feature::Trig.bit() | Feature::Atrig.bit() | Feature::Brackets.bit());
        assert!(un.contains(&(MACRO0 + 3))); // sin(atan)
        assert!(!un.contains(&MACRO0)); // sec(atan) needs rtrig
    }
}
