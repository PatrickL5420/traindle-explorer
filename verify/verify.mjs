// Verify solver output against the real Traindle game rules (mathjs 15.2).
//
//   node verify/verify.mjs output/solutions.json [--summary output/summary.json] [--ref ref_solutions.json]
//   node verify/verify.mjs runs/<id>/solutions.json
//
// Uses the same checks as the web UI (web/verify-core.js): the game's validExpression /
// hasFourNumbers / evaluateExpression (generalised to n digits and any target), exact
// equality, no float artefacts (underflow to 0, negligible values: checked against 64-digit
// BigNumbers), no leading-zero literals, and the declared tier must match the tier derived
// from the expression's mathjs parse tree under the run's tier config. The config comes
// from the summary.json next to the solutions file. Exits non-zero on any failure.

import { create, all } from "mathjs";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { verifyRun, featuresOf } = require("../web/verify-core.js");
const math = create(all);
const big = create(all, { number: "BigNumber", precision: 64 });

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : null;
};
const file = args.find((a, i) => !a.startsWith("--") && !(i > 0 && args[i - 1].startsWith("--"))) ?? "output/solutions.json";
const summaryFile = opt("--summary") ?? path.join(path.dirname(file), "summary.json");
const refFile = opt("--ref");

const sols = JSON.parse(fs.readFileSync(file, "utf8"));
if (!fs.existsSync(summaryFile)) {
  console.error(`no ${summaryFile}: pass --summary with the run's summary.json (it carries the tier config)`);
  process.exit(2);
}
const summary = JSON.parse(fs.readFileSync(summaryFile, "utf8"));
const cfg = summary.config;
const run = { digits: cfg.digits, target: cfg.target, tiers: cfg.tiers };
const total = 10 ** cfg.digits;

const report = verifyRun(math, run, sols, null, 500, big);
const hist = new Array(cfg.tiers.length + 1).fill(0);
for (const s of sols) hist[s.tier] = (hist[s.tier] ?? 0) + 1;

console.log(`checked ${report.checked} solutions + ${report.alts_checked} alternatives from ${file} in ${report.elapsed_ms} ms (mathjs ${report.mathjs_version})`);
console.log(`${cfg.digits} digits, target ${cfg.target}: ${sols.length} / ${total} solved, ${total - sols.length} unsolved`);
console.log(`max |value - target| = ${report.max_abs_error.toExponential(2)}`);
console.log("tier histogram:");
cfg.tiers.forEach((t, i) => console.log(`  ${String(i + 1).padStart(2)} ${t.name.padEnd(16)} ${String(hist[i + 1]).padStart(6)}`));

if (cfg.digits === 4 && Number(cfg.target) === 10) {
  const byNum = new Map(sols.map((s) => [s.number, s]));
  const HARD = "0067 0607 0670 0706 0076 0000 0001 0002 0020 0021 0102 0200 0201 2100 2101 0760 6671 6670 0667 0676 0677 0761 6177 7677".split(" ");
  console.log("\npreviously 'hard' numbers:");
  for (const n of HARD) {
    const s = byNum.get(n);
    console.log(`  ${n}  T${String(s?.tier ?? "-").padEnd(2)}  ${s?.solution ?? "(unsolved)"}`);
  }
  if (refFile) {
    // tier of a reference expression under this run's tier config
    const tierOf = {};
    cfg.tiers.forEach((t, i) => t.features.forEach((f) => (tierOf[f] = i + 1)));
    const refTier = (e) => {
      let t = 1;
      for (const f of featuresOf(math.parse(e))) t = Math.max(t, f.startsWith("?") ? cfg.tiers.length + 1 : tierOf[f] ?? cfg.tiers.length + 1);
      if (/\d\d/.test(e)) t = Math.max(t, tierOf.concat ?? 1); // ref literals may have leading zeros
      return t;
    };
    let lower = 0, higher = 0, same = 0;
    const worse = [];
    for (const r of JSON.parse(fs.readFileSync(refFile, "utf8"))) {
      const s = byNum.get(r.number);
      if (!s) continue;
      const rt = refTier(String(r.solution));
      if (s.tier < rt) lower++;
      else if (s.tier > rt) {
        higher++;
        worse.push(`${r.number}: ours T${s.tier} "${s.solution}" vs ref T${rt} "${r.solution}"`);
      } else same++;
    }
    console.log(`\nvs ${refFile}: ${lower} at a lower tier, ${same} same tier, ${higher} at a higher tier`);
    for (const w of worse.slice(0, 30)) console.log("  " + w);
  }
}

if (!report.ok) {
  console.log(`\nFAILURES (${report.failure_count}):`);
  for (const f of report.failures.slice(0, 50)) console.log(`  ${f.number}${f.alt ? " (alt)" : ""}: ${f.solution}\n      ${f.errors.join("; ")}`);
  process.exit(1);
}
console.log(`\nall ${report.checked} solutions valid under the game rules and exactly equal to ${cfg.target}`);
