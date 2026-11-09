// Verify solver output: every solution must be mathematically true and accepted by the
// real Traindle game (which evaluates with mathjs 15.2).
//
//   node verify/verify.mjs [output]            a run folder (CLI output or runs/<id>)
//   node verify/verify.mjs runs/<id> --skip-alts --ref ref_solutions.json
//   node verify/verify.mjs solutions.json --summary summary.json   (game-format JSON)
//
// Uses the same checks as the web UI (web/verify-core.js): the game's validExpression /
// hasFourNumbers / evaluateExpression (generalised to n digits and any target), exact
// equality, no operation that is undefined in real arithmetic even where mathjs returns a
// value (x % 0, 0^0, division by zero, acot(-0)), no float artefacts (underflow to 0,
// negligible values: checked against 64-digit BigNumbers), no leading-zero literals, and
// the declared tier must match the tier derived from the expression's parse tree under the
// run's tier config (from summary.json). Alternatives (alts.bin) are checked too unless
// --skip-alts is given. Exits non-zero on any failure.

import { create, all } from "mathjs";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
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
const VALUE_OPTS = ["--summary", "--ref"];
const input = args.find((a, i) => !a.startsWith("--") && !(i > 0 && VALUE_OPTS.includes(args[i - 1]))) ?? "output";
const refFile = opt("--ref");
const isDir = fs.statSync(input).isDirectory();
const file = input;
const summaryFile = opt("--summary") ?? path.join(isDir ? input : path.dirname(input), "summary.json");
if (!fs.existsSync(summaryFile)) {
  console.error(`no ${summaryFile}: pass --summary with the run's summary.json (it carries the tier config)`);
  process.exit(2);
}

/** A run folder: solutions.txt (line i = puzzle i), grid.bin (tiers), alts.bin (blocks). */
function readRunFolder(dir, digits, withAlts) {
  const texts = fs.readFileSync(path.join(dir, "solutions.txt"), "utf8").split("\n");
  const grid = fs.readFileSync(path.join(dir, "grid.bin"));
  const n = grid.readUInt32LE(4);
  const sols = [];
  for (let i = 0; i < n; i++) {
    if (!texts[i]) continue;
    sols.push({ number: String(i).padStart(digits, "0"), solution: texts[i], tier: grid[16 + i] });
  }
  const altsPath = path.join(dir, "alts.bin");
  if (withAlts && fs.existsSync(altsPath)) {
    const a = fs.readFileSync(altsPath);
    const block = a.readUInt32LE(8), nblocks = a.readUInt32LE(12);
    const byIndex = new Map(sols.map((s) => [Number(s.number), s]));
    for (let b = 0; b < nblocks; b++) {
      const off = Number(a.readBigUInt64LE(16 + 12 * b)), len = a.readUInt32LE(16 + 12 * b + 8);
      const entries = JSON.parse(zlib.inflateSync(a.subarray(off, off + len)).toString("utf8"));
      entries.forEach((alts, k) => {
        const s = byIndex.get(b * block + k);
        if (s && alts.length) s.alts = alts.map((x) => x[1]);
      });
    }
  }
  return sols;
}

const summary = JSON.parse(fs.readFileSync(summaryFile, "utf8"));
const cfg = summary.config;
const sols = isDir ? readRunFolder(input, cfg.digits, !args.includes("--skip-alts")) : JSON.parse(fs.readFileSync(input, "utf8"));
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
console.log(`\nall ${report.checked} solutions mathematically exact (= ${cfg.target}) and accepted by the game`);
