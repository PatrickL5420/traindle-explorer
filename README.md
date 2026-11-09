# Traindle Explorer

An exercise in mathematics based on the Traindle game. For every puzzle of n digits (1–5), it
finds the simplest expression that uses the digits **in order** and is **mathematically equal**
to the target (default: 4 digits, target 10, as in the game). Operators are grouped into
**tiers** that are tried in order, and each solution's tier doubles as a difficulty rating.

Correctness means real-number arithmetic, not whatever a calculator prints. The game's
evaluator (mathjs 15.2) is not the source of truth: where it bends the maths (`x % 0 = x`,
`0^0 = 1`, float underflow), those expressions are rejected (see [The maths](#the-maths)). The
game still sets the puzzle format: digits in order, the allowed functions, and the written
notation. Every solution is also accepted by the game.

**Default game:** all 10000 numbers solved in ~27 s on an i7-12700K (20 threads; a
[native-CPU build](#faster-builds-for-your-own-cpu) is ~15% faster), peak memory ~1.3 GB.
5 digits (100,000 puzzles) takes ~22 s. Every solution is re-evaluated in 256-bit precision and
cross-checked with the game's evaluator.

| tier | features (default) | 4 digits → 10 |
|---|---|---|
| 1 | `+ - * /`, leading unary minus, no brackets | 5354 |
| 2 | brackets (nested) | 1537 |
| 3 | `^` | 945 |
| 4 | concatenation (joining digits, e.g. `12`) | 365 |
| 5 | `!` | 1487 |
| 6 | `%` (modulo) | 282 |
| 7 | `sqrt()` | 18 |
| 8 | `cbrt()` | 1 |
| 9–10 | `combinations()`, `permutations()` | 0 |
| 11 | trig (all 12 whitelisted functions) | 11 |

A puzzle's solution comes from the earliest tier that can solve it. Within that tier it is the
cheapest by operator weight, then the shortest string. Its tier is the highest tier any part of
the expression needs.

Literal rules: joined digits need the concatenation feature, and a multi-digit literal may not
start with 0 (`06` is just `0 + 6` in disguise). Each joined digit costs a little (weight
`concat`, default 4, below every operator).

## The maths

Expressions are evaluated over the real numbers. An operation that is undefined there is never
used, even where the game's evaluator returns a value:

| expression | here | mathjs (the game) |
|---|---|---|
| `x / 0` | undefined | `Infinity` (so `atan(1/0)` "is" π/2) |
| `x % 0` | undefined | `x` |
| `0^0` | undefined | `1` |
| `0^(-n)` | undefined | `Infinity` |
| `(-8)^(1/3)` | not real | complex |
| `sqrt(-x)`, `(-n)!`, `asin(2)`, `asec(0.5)` | undefined | complex or error |
| `acot(-0)` | π/2 (it's `acot(0)`) | −π/2 (signed zero), so rejected |

Conventions where mathematics offers a choice:

- `a % b` is the floored modulo `a − b·⌊a/b⌋`, so the result takes the sign of `b`: `-7 % 3 = 2`.
- `n!` is defined for non-negative integers. Non-integers via the gamma function are only
  allowed with `--gamma` (off by default).
- `combinations(n, k)` and `permutations(n, k)` need integers with 0 ≤ k ≤ n.
- `cbrt` is the real cube root (`cbrt(-8) = -2`). A negative base needs an integer exponent.
- Trig works in radians. Inverse functions return principal values, with
  `asec(x) = acos(1/x)`, `acsc(x) = asin(1/x)` and `acot(x) = atan(1/x)` (values in (−π/2, 0) ∪ (0, π/2]).
- Notation follows the game: `^` is right-associative and binds tighter than unary minus
  (`-2^2 = -4`), and `!` binds tighter than `^`.

**Equal means equal.** Floating point can make a false expression look true. For example,
`9^(-7!)` ≈ 4·10⁻⁴⁸¹⁰ underflows to exactly 0, so `7^9^(-7!) + 9` "equals" 10 in mathjs.
Such cases are rejected:
- a product, quotient or power of nonzero numbers is never treated as 0;
- no sub-expression may be negligibly small (|v| < 10⁻⁹);
- every candidate is re-evaluated in 256-bit precision (~77 significant digits). At every node
  the 64-bit and 256-bit values must agree, and the result must match the target exactly at
  that precision.

That last check is very strong numerical evidence, not a symbolic proof. Identities like
`sec(atan(√8)) = 3` are confirmed to ~77 digits rather than derived.

## Building

Traindle Explorer is a single Rust program. Build it from source on Windows, macOS or Linux; it
needs about 2 GB of free RAM for the default game, more for deep tiers on 5 digits.

### 1. Install the tools

**Rust** (stable) comes from [rustup](https://rustup.rs). It also needs a C compiler, for the
`mimalloc` memory allocator:

- **Windows:** run `rustup-init.exe` from [rustup.rs](https://rustup.rs) and accept its offer to
  install the Visual Studio C++ Build Tools (or install them yourself: *Desktop development with
  C++*).
- **macOS:** install the compiler with `xcode-select --install`, then rustup:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Linux:** install a compiler first (Debian/Ubuntu: `sudo apt install build-essential curl`;
  Fedora: `sudo dnf install gcc curl`), then rustup with the same `curl … | sh` line as macOS.

Open a new terminal afterwards and check that `cargo --version` works.

**Node.js 18+** ([nodejs.org](https://nodejs.org)) is optional. You only need it for the
command-line verifiers in `verify/`, or to serve mathjs locally instead of from a CDN.

### 2. Build

```bash
git clone https://github.com/PatrickL5420/traindle-explorer.git
cd traindle-explorer
cargo build --release
```

The first build downloads dependencies and takes a few minutes. The program ends up in
`target/release/traindle-explorer` (`traindle-explorer.exe` on Windows).

Optional checks:

```bash
cargo test --release                     # unit tests
cd verify && npm install && cd ..        # local mathjs for the verifiers and the web UI
```

### 3. Run

```bash
./target/release/traindle-explorer serve     # web UI, opens http://127.0.0.1:7878/
./target/release/traindle-explorer           # default game on the command line -> output/
```

On Windows, write `.\target\release\traindle-explorer.exe` in PowerShell or `cmd`.

Run it from the repository folder. Saved runs (`runs/`), command-line output (`output/`) and the
optional `web/` and `verify/node_modules` overrides are found relative to the current folder.
The page itself is built into the program, so the server works from anywhere. Without
`verify/node_modules`, the page loads mathjs from a CDN, which needs internet access.

### Faster builds for your own CPU

The default build runs on any machine. To optimise for the CPU you build on (about 15% faster
here, with identical results), build with:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release          # macOS / Linux / Git Bash
$env:RUSTFLAGS="-C target-cpu=native"; cargo build --release    # PowerShell
```

A binary built this way may crash with "illegal instruction" on other computers, so don't share
it.

### Troubleshooting

- **`linker 'link.exe' not found` (Windows) or `cc not found` / `linker 'cc' not found`:** the C
  compiler from step 1 is missing.
- **Port 7878 in use:** `traindle-explorer serve --port 8080`.
- **The browser doesn't open:** use `--no-open` and open the printed address yourself.
- **Out of memory on 5 digits:** close other programs, or limit the tiers with `--max-tier N`.

## Web UI

Start it with `traindle-explorer serve` (see [Building](#building)); it opens
http://127.0.0.1:7878/. Options: `--port N`, `--runs-dir DIR`, `--no-open`.

- **New run:** a name, digit count (1–5) and target (any number, e.g. 24, -3 or 0.5).
- **Target sweep:** enter several targets to run one after another, e.g. `1..20`, `0..1..0.25`
  (start..end..step) or `10, 24, 100` (at most 200). The Run button turns into *Sweep ×N*.
  - Each target is saved as a normal run, and they are grouped under one row in the runs table.
  - The sweep chart shows one 100% stacked bar per target (share of puzzles per tier). It can be
    ordered by target or by difficulty (mean tier, with unsolved as one past the last tier).
  - Click a bar to open that target's run. Every run of the sweep is verified afterwards.
- **Tier editor:**
  - Drag features between tiers, or click one to move it.
  - Reorder tiers with the arrows, add or remove tiers, and drop unwanted features into *Not used*.
  - Function-style features (`!`, `sqrt`, `cbrt`, combinations, permutations, trig) need brackets
    in the same or an earlier tier.
  - Tiers before brackets are solved as flat expressions with normal precedence.
  - Operator weights are under *Advanced*.
  - **Presets** replace the tiers:
    - *Injective only* allows one-to-one functions only: unary minus, sqrt, cbrt and inverse
      trig. It drops `!`, combinations/permutations and sin/cos/tan/sec/csc/cot.
    - *No external numbers* removes every function that hides a constant (sqrt, cbrt, `!`,
      combinations, permutations, trig), leaving only operators.
  - **Trig search depth** (1–3 rounds, default 2) is set in the run form.
  - **Rarity ±** (default 10, about one operation) is the rarity window. Besides the best solution,
    the solver keeps every verified solution costing at most best + this. That gives each puzzle a
    rarity count and up to 4 alternatives. 0 turns it off. On the default game it adds about 6% to
    the run time and never changes the best solution.
- **Progress:**
  - *Generating* shows puzzles solved so far.
  - The thinner bar shows the current round and stage (building value sets / joining).
  - *Cancel* stops a run and keeps its partial results.
- **Verification:**
  - Each finished or opened run is checked in the browser, in a Web Worker, against the rules
    in [The maths](#the-maths): no undefined operations, no float artefacts, exact equality.
  - It also checks that the game accepts each solution: its input validation, no leading zeros,
    and the value the game's evaluator (mathjs 15.2) computes. The declared tier must match the
    tier recomputed from the parse tree.
  - The report is saved with the run.
- **Runs** are saved under `runs/<id>/` (`config.json`, `solutions.json`, `summary.json`,
  `verify.json`). Pick a run as **A** to view it. Pick a second run with the same digit count as
  **B** to compare. ⤓ downloads the game-format `solutions.json`.
- **Heatmap:** one cell per puzzle, coloured by tier, with unsolved in red.
  - 4 digits = 100×100, with rows = first two digits.
  - 5 digits = one 100×100 panel per first digit.
  - Hover for the solution. Click legend entries to filter; several can be combined.
  - **Colour by** *Tier*, *Cost* (sum of operator weights), *Length* (characters) or *Rarity*
    (how few solutions exist near the best one; darker = rarer). The metric modes use up to 7
    quantile bins. Rarity counts distinct value paths the solver found, so it is a relative index,
    not an exact count. Runs saved before this feature only have *Tier* and *Length*.
  - In compare mode, colour shows how each puzzle's tier moved from A to B: blue = easier in B,
    red = harder, gray = same.
  - Compare mode also shows tier-weighted totals: harder puzzles, the tier steps they gained,
    easier puzzles, the tier steps they lost, and the net per puzzle.
  - *Changed puzzles* lists every puzzle whose tier moved, biggest moves first, with both
    solutions. It follows the legend filter; click a row to open its trees.
- **Expression tree:** click a cell to draw its solution as a tree.
  - Boxes hold the numbers.
  - Arrows carry values up to the result, labelled with the operation and coloured by its tier.
  - Joined-digit boxes are tinted with the concat tier.
  - In compare mode A and B are shown side by side.
  - Numbered pills switch between the best solution and its alternatives.
- The URL hash keeps the view (`#a=<run>&b=<run>&sel=0677&c=count`, or `#sweep=<id>`), so a view can be bookmarked. The
  theme follows the OS, or use the header toggle.

The page lives in `web/`, which is embedded in the binary. If a `web/` folder exists in the
working directory it is served from disk instead, so the page can be edited without rebuilding.

## Command line

```bash
./target/release/traindle-explorer                              # default game -> output/
./target/release/traindle-explorer --digits 3 --target 24
./target/release/traindle-explorer --config runs/<id>/config.json   # re-run a saved config
./target/release/traindle-explorer --only 0000,0677,7677 --stats
```

| flag | meaning |
|---|---|
| `--config FILE` | a saved `config.json` (tiers, weights, digits, target); flags below override it |
| `--digits N`, `--target X` | puzzle size (1–5) and target |
| `--max-tier N` | only use the first N tiers |
| `--profile default\|thorough` | search budgets; `thorough` takes ~115 s on the default game and currently finds identical solutions |
| `--concat-cost N` | weight per joined digit |
| `--gamma` | allow factorial of non-integers via the gamma function, `x! = Γ(x+1)` |
| `--trig-rounds N` | trig search depth, 1–3 (default 2). Round 3 (unfiltered inverse trig, biggest budget) is a slow last resort that can take minutes |
| `--alt-slack N` | rarity window: also keep verified solutions costing at most best + N (default 10, 0 = off) |
| `--max-abs X` | largest intermediate magnitude (default 1e7) |
| `--threads N` | worker threads (default: all) |

Output: `output/solutions.json` (`[{ "number", "solution", "tier", "cost", "count", "alts" }]`) and `output/summary.json`
(stats plus the config, which the verifier reads).

## Verification

```bash
node verify/verify.mjs output/solutions.json --ref path/to/ref_solutions.json
node verify/verify.mjs runs/<id>/solutions.json
./target/release/traindle-explorer --emit-expr-tests 50000 --seed 1
node verify/exprcheck.mjs output/expr_tests.json
cargo test --release
```

- `verify.mjs` and the web UI share `web/verify-core.js`. It checks two things independently
  of the solver:
  - **The maths:** no operation that is undefined over the reals (`x % 0`, `0^0`, division by
    zero, `acot(-0)`), no float artefacts (underflow "zeros", negligible values; every `^` is
    re-checked with 64-digit BigNumbers), and exact equality with the target.
  - **The game:** verbatim copies of the game's `validExpression` / `hasFourNumbers` /
    `evaluateExpression` (generalised to n digits and any target), no leading zeros, and the
    tier recomputed from the parse tree.
- `exprcheck.mjs` fuzzes random Rust-rendered expressions against mathjs. It checks that the game
  reads our notation the same way (precedence/parens) and computes the same 64-bit value. Rust
  rejecting something mathjs accepts (e.g. `x % 0`) is expected. Rare differences of ~1e-11
  can appear where both sides round differently on long inexact chains (e.g. `sin` inside a
  `%`). Neither value is the exact one, and solutions are protected by the 256-bit check.

## How it works

1. **Flat tiers** (`flat.rs`). Every tier before brackets is enumerated directly: concatenation
   splits × operator choices × an optional leading minus. Evaluation follows the game's
   precedence (`^` right-associative, then `* / %`, then `+ -`).
2. **Cross-number memoisation** (`cache.rs`). A substring's set of reachable values depends only
   on its digits, so each round builds at most 10 + 100 + … + 10^(n−1) sets, shared read-only by
   all puzzles. Only substrings that still-unsolved puzzles need are built.
3. **Value sets** (`span.rs`). Each value keeps its cheapest expression.
   - Binary combine visits children in cost order and prunes on the round's cost cap.
   - The unary closure runs in cost order (Dial buckets).
   - Dedup uses a relative quantised key in a hashbrown table.
4. **Root join** (`root.rs`), a meet-in-the-middle step. For each split and each *pretarget* t
   (a value with a unary chain to the target, such as target² via `sqrt`), it iterates the
   smaller side and hash-probes the other for the exact partner:
   - `+ - * /`: solved directly for the partner;
   - `^`: logarithms;
   - `%`: divisors via a prime sieve;
   - combinations/permutations: (n, k) tables.

   Branch-and-bound uses a shared atomic best cost.
5. **Trig.** Inverse trig is kept only for rational multiples of π, and forward trig is applied
   only to such angles. 13 fused macros like `sec(atan(x)) = √(1+x²)` and `cot(atan(x)) = 1/x` act
   as single steps. A macro is only used when both of its function groups are enabled.
6. **Rounds and budgets** (`config.rs`). Tiers without unary growth are searched exhaustively.
   Deeper tiers get two or three rounds with growing cost budgets, scaled down for 1–2 digit
   puzzles, where tiny substrings would otherwise explode.
7. **Exactness.** The search never accepts a zero produced by underflow: a power, product or
   quotient of nonzero values is never 0, and powers outside [1e-9, max] are skipped up front.
   A candidate is verified the moment it would become a puzzle's best: it is re-evaluated in
   f64 (the game's arithmetic) and in 256-bit precision (`astro-float`, the mathematical value).
   Every node must agree between the two, and the result must match the exact decimal target. Only verified
   candidates tighten the pruning bound, so a cheap fake can never hide a genuine solution.

**Why CPU, not GPU:** the work is hash inserts and lookups plus branchy f64 domain checks, not
FLOPs. Consumer GPUs run FP64 at 1/64 rate, and wgpu has no f64. Rust + rayon finishes the first
seven tiers of the default game in under 0.5 s; almost all of the runtime is the deep search for
the last ~12 puzzles.

## Layout

```
src/config.rs     features, tier list, weights, round schedule
src/ops.rs        operator codes; real arithmetic and domains (floored %, undefined x%0, 0^0)
src/flat.rs       bracket-free tiers (enumeration with precedence)
src/span.rs       value-set builder (combine, unary closure, freeze)
src/cache.rs      substring memoisation, parallel level builds
src/pretarget.rs  unary chains that end at the target
src/root.rs       root join (hash probes, %, C/P), candidate selection
src/expr.rs       tree rebuild, minimal-paren rendering, feature detection, f64 + 256-bit eval
src/driver.rs     round loop, progress, stats
src/progress.rs   live progress + cancellation (shared with the server)
src/runs.rs       run ids, summary, saving
src/server.rs     `serve`: local web UI + JSON API
web/              index.html (UI), verify-core.js (shared checks), verify-worker.js
verify/           Node checks: the maths, and acceptance by the game (mathjs 15.2)
```
