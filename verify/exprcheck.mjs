// Cross-check the Rust renderer + f64 arithmetic against mathjs, the game's evaluator.
//
//   traindle-explorer --emit-expr-tests 20000      (writes output/expr_tests.json)
//   node verify/exprcheck.mjs output/expr_tests.json
//
// Every random expression was rendered and evaluated by Rust. Where Rust gives a value,
// mathjs must parse the text to the same value, or the game would disagree with us: a
// mismatch is a rendering (precedence/parens) or arithmetic bug. Rust rejects more than
// mathjs (operations undefined in real arithmetic such as x % 0 or 0^0, infinite
// intermediates); that is expected, never the other way round.

import { evaluate } from "mathjs";
import fs from "node:fs";

const file = process.argv[2] ?? "output/expr_tests.json";
const tests = JSON.parse(fs.readFileSync(file, "utf8"));

let ok = 0, stricter = 0;
const mismatches = [];
const stricterSamples = [];

for (const { expr, value } of tests) {
  let m = null;
  try {
    const r = evaluate(expr);
    if (typeof r === "number" && Number.isFinite(r)) m = r;
  } catch {
    m = null;
  }
  if (value === null) {
    if (m === null) ok++;
    else {
      stricter++;
      if (stricterSamples.length < 5) stricterSamples.push(`${expr}  (mathjs ${m})`);
    }
  } else if (m === null) {
    mismatches.push(`${expr}: rust ${value}, mathjs invalid`);
  } else {
    const tol = Math.max(1e-12, 1e-9 * Math.abs(m));
    if (Math.abs(m - value) <= tol) ok++;
    else mismatches.push(`${expr}: rust ${value}, mathjs ${m}`);
  }
}

console.log(`${tests.length} expressions: ${ok} agree, ${stricter} rejected only by rust, ${mismatches.length} mismatches`);
for (const s of stricterSamples) console.log("  stricter: " + s);
for (const s of mismatches.slice(0, 30)) console.log("  MISMATCH: " + s);
process.exit(mismatches.length ? 1 : 0);
