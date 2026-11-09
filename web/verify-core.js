// Shared verification logic: runs in the browser (Web Worker / page) and in Node
// (verify/verify.mjs). Pass in the mathjs instance; nothing here imports it.
//
// Two questions are checked for every solution:
// 1. Is it mathematically true? It must use only operations defined over the reals
//    (no x % 0, 0^0, division by zero, ...) and equal the target exactly. Float
//    artefacts don't count: an underflow "zero" or a negligible value is rejected, and
//    powers are re-checked with 64-digit BigNumbers.
// 2. Does the game accept it? The game parses and evaluates with mathjs, so mathjs
//    is used for the notation (parse tree, features, tiers) and to confirm the game
//    computes the target. mathjs is not the judge of correctness: where it bends the
//    maths (x % 0 = x, 0^0 = 1, acot(-0) = -pi/2, atan(1/0) = pi/2) we reject.
//
// The game's own input checks (Waffles-traindle/shared/gameUtils.ts) are copied verbatim
// and generalised from "4 digits, equals 10" to "n digits, equals target".
(function (root) {
  "use strict";

  // ---- copied from gameUtils.ts
  const allowedFunctions = [
    "sqrt", "cbrt", "gcd", "lcm", "mod", "factorial", "combinations", "permutations",
    "sin", "cos", "tan", "csc", "cot", "sec", "acos", "asin", "atan", "acsc", "acot", "asec", "dot",
  ];
  const allowedPattern = new RegExp(`\\b(?:${allowedFunctions.join("|")})\\b`, "g");

  function validExpression(s) {
    if (/[.[\];=\n?]/.test(s)) return false;
    if (s.includes("__proto__") || s.includes("constructor")) return false;
    if (/[a-zA-Z]/.test(s.replace(allowedPattern, ""))) return false;
    return true;
  }

  function hasNumbers(expression, numbers) {
    const found = expression.match(/\d/g)?.map(Number);
    return !!found && found.length === numbers.length && found.every((n, i) => n === numbers[i]);
  }

  // evaluateExpression's display rounding
  function gameRound(result) {
    if (Math.abs(result) >= 1e9) return result.toExponential(2);
    return (Math.round(result * 100) / 100).toString();
  }
  // ----

  const OP_FEATURE = {
    add: "add", subtract: "sub", multiply: "mul", divide: "div",
    pow: "pow", mod: "mod", unaryMinus: "neg", factorial: "fact",
  };
  const FN_FEATURE = {
    sqrt: "sqrt", cbrt: "cbrt", combinations: "comb", permutations: "perm",
    sin: "trig", cos: "trig", tan: "trig",
    asin: "atrig", acos: "atrig", atan: "atrig",
    sec: "rtrig", csc: "rtrig", cot: "rtrig",
    asec: "artrig", acsc: "artrig", acot: "artrig",
  };

  /** Feature a mathjs node introduces (null = none, "?x" = not a solver feature). */
  function nodeFeature(node) {
    switch (node.type) {
      case "OperatorNode":
        return OP_FEATURE[node.fn] ?? `?${node.op}`;
      case "FunctionNode":
        return FN_FEATURE[node.fn.name] ?? `?${node.fn.name}()`;
      case "ParenthesisNode":
        return "brackets";
      case "ConstantNode":
        // leading-zero literals are rejected separately, so >= 10 means joined digits
        return typeof node.value === "number" && Math.abs(node.value) >= 10 ? "concat" : null;
      default:
        return `?${node.type}`;
    }
  }

  function featuresOf(tree) {
    const set = new Set();
    tree.traverse((n, path) => {
      // a function call's name is a SymbolNode child at path "fn": not a variable
      if (n.type === "SymbolNode" && path === "fn") return;
      const f = nodeFeature(n);
      if (f) set.add(f);
    });
    return set;
  }

  const NEGLIGIBLE = 1e-9;

  /**
   * Why an expression is not mathematically valid, even if mathjs evaluates it.
   * - Undefined operations mathjs gives a value anyway: x % 0 (mathjs: x), 0^0
   *   (mathjs: 1), anything divided by 0 or another non-finite intermediate (mathjs:
   *   Infinity, so atan(1/0) "is" pi/2), and acot of a negative zero (mathjs: -pi/2).
   * - Float artefacts the final value can't reveal. mathjs evaluates in 64-bit floats,
   *   so 9^(-7!) (really 4e-4810) underflows to exactly 0 and 7^9^(-7!) + 9 "equals" 10.
   *   No sub-expression may be a negligible nonzero value (|v| < 1e-9); a zero product /
   *   quotient / power of nonzero operands is underflow; and every ^ sub-expression is
   *   re-evaluated with 64-digit BigNumbers (`big`, optional).
   * The Rust solver applies the same rules.
   */
  function numericIssues(tree, big) {
    const issues = [];
    tree.traverse((n) => {
      if (issues.length >= 2 || (n.type !== "OperatorNode" && n.type !== "FunctionNode")) return;
      let v;
      try { v = n.evaluate(); } catch { return; }
      if (typeof v !== "number") return;
      const txt = n.toString();
      let args = [];
      try { args = n.args.map((a) => a.evaluate()); } catch { return; }
      const op = n.type === "OperatorNode" ? n.fn : n.fn.name;
      if (op === "mod" && args[1] === 0) { issues.push(`${txt}: x % 0 is undefined (mathjs returns x)`); return; }
      if (op === "pow" && args[0] === 0 && args[1] <= 0) { issues.push(`${txt}: 0^${args[1] === 0 ? "0" : "(negative)"} is undefined${args[1] === 0 ? " (mathjs returns 1)" : ""}`); return; }
      if (op === "acot" && Object.is(args[0], -0)) { issues.push(`${txt}: acot of a negative zero (mathjs returns -pi/2; acot(0) = pi/2)`); return; }
      if (args.some((a) => typeof a === "number" && !Number.isFinite(a))) { issues.push(`${txt} uses an undefined or infinite value (e.g. division by 0)`); return; }
      if (!Number.isFinite(v)) return;
      if (v !== 0 && Math.abs(v) < NEGLIGIBLE) {
        issues.push(`${txt} is a negligible ${v.toExponential(2)}, used as if it were 0`);
        return;
      }
      if (v === 0 && n.type === "OperatorNode" && ["multiply", "divide", "pow"].includes(n.fn)) {
        const nonzero = n.fn === "multiply" ? args.every((x) => x !== 0) : args[0] !== 0;
        if (nonzero) {
          let real = "";
          if (big) try { const r = big.evaluate(txt); if (!r.isZero()) real = `; its real value is ${r.toExponential(3)}`; } catch {}
          issues.push(`${txt} underflows to 0 in floating point${real}`);
          return;
        }
      }
      if (big && n.type === "OperatorNode" && n.fn === "pow") {
        let b;
        try { b = big.evaluate(txt); } catch { return; }
        if (!big.isBigNumber(b)) return;
        if (!b.isZero() && b.abs().lt(NEGLIGIBLE)) {
          issues.push(`${txt} is really ${b.toExponential(3)}, a negligible value`);
        } else {
          const scale = Math.max(1, Math.abs(b.toNumber()));
          if (Math.abs(b.toNumber() - v) > 1e-9 * scale) issues.push(`${txt} is ${v} in floats but really ${b.toPrecision(12)}`);
        }
      }
    });
    return issues;
  }

  /**
   * Build a checker for one run. `run` = { digits, target, tiers: [{name, features}] }.
   * check(entry) -> { ok, errors: [...], value, tier }
   */
  function makeVerifier(math, run, big) {
    if (big === undefined && typeof math.create === "function" && math.all) {
      try { big = math.create(math.all, { number: "BigNumber", precision: 64 }); } catch { big = null; }
    }
    const tierOf = {};
    run.tiers.forEach((t, i) => t.features.forEach((f) => (tierOf[f] = i + 1)));
    const target = Number(run.target);
    const tol = 1e-9 * Math.max(1, Math.abs(target));
    const shownTarget = Number(gameRound(target));

    return function check(entry) {
      const errors = [];
      const e = entry.solution;
      const digits = Array.from(entry.number, Number);
      let value = null;
      let tier = 1;
      if (!validExpression(e)) errors.push("game validExpression() rejects it");
      if (!hasNumbers(e, digits)) errors.push("digits are not exactly " + entry.number + " in order");
      const lead0 = (e.match(/\d+/g) ?? []).filter((lit) => lit.length > 1 && lit[0] === "0");
      if (lead0.length) errors.push("leading-zero literal " + lead0.join(", "));
      try {
        const tree = math.parse(e);
        const v = tree.evaluate();
        if (typeof v !== "number" || !Number.isFinite(v)) {
          errors.push("does not evaluate to a real finite number");
        } else {
          value = v;
          if (Math.abs(v - target) > tol) errors.push(`not exactly ${run.target}: ${v}`);
          if (Number(gameRound(v)) !== shownTarget) errors.push(`game shows ${gameRound(v)}, not ${gameRound(target)}`);
        }
        for (const msg of numericIssues(tree, big)) errors.push(msg);
        for (const f of featuresOf(tree)) {
          if (f.startsWith("?")) errors.push("uses " + f.slice(1) + ", which no tier provides");
          else if (!tierOf[f]) errors.push("uses " + f + ", which is not in any tier");
          else tier = Math.max(tier, tierOf[f]);
        }
        if (entry.tier != null && entry.tier !== tier) errors.push(`declared tier ${entry.tier}, expression needs tier ${tier}`);
      } catch (err) {
        errors.push("mathjs error: " + err.message);
      }
      return { ok: errors.length === 0, errors, value, tier };
    };
  }

  /** Verify a whole run, reporting progress every `every` entries. */
  function verifyRun(math, run, solutions, onProgress, every = 500, big) {
    const t0 = Date.now();
    const check = makeVerifier(math, run, big);
    const failures = [];
    let maxErr = 0;
    let alts = 0;
    solutions.forEach((s, i) => {
      const r = check(s);
      if (!r.ok) failures.push({ number: s.number, solution: s.solution, errors: r.errors });
      else maxErr = Math.max(maxErr, Math.abs(r.value - Number(run.target)));
      // alternatives: same checks, except their tier is not declared
      for (const a of s.alts ?? []) {
        alts++;
        const ra = check({ number: s.number, solution: a, tier: null });
        if (!ra.ok) failures.push({ number: s.number, solution: a, alt: true, errors: ra.errors });
      }
      if (onProgress && (i + 1) % every === 0) onProgress(i + 1, solutions.length);
    });
    if (onProgress) onProgress(solutions.length, solutions.length);
    return {
      ok: failures.length === 0,
      checked: solutions.length,
      alts_checked: alts,
      failure_count: failures.length,
      failures: failures.slice(0, 200),
      max_abs_error: maxErr,
      mathjs_version: math.version,
      elapsed_ms: Date.now() - t0,
      verified_at: new Date().toISOString(),
    };
  }

  const api = { validExpression, hasNumbers, gameRound, nodeFeature, featuresOf, numericIssues, makeVerifier, verifyRun };
  root.TraindleVerify = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof self !== "undefined" ? self : globalThis);
