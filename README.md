# Traindle Explorer

**The game:** you're given four digits, like `2 4 7 9`. Using them in that order, with any
mathematical operators you like, write an expression equal to 10. For example,
`2 * 4 - 7 + 9 = 10`.

**This program** finds the simplest solution for every one of the 10,000 puzzles (`0000`–`9999`)
and lets you explore them. You can also change the rules: any target, 1–5 digits, and which
operators are allowed.

Operators are grouped into **tiers**, tried from easiest to hardest. A puzzle's tier is the
easiest tier that can solve it, which makes it a natural difficulty rating:

| tier | operators | puzzles (4 digits → 10) |
|---|---|---|
| 1 | `+ - * /` | 5354 |
| 2 | brackets | 1537 |
| 3 | `^` | 945 |
| 4 | joining digits (`1` `2` → `12`) | 365 |
| 5 | `!` | 1487 |
| 6 | `%` (modulo) | 282 |
| 7 | `sqrt` | 18 |
| 8 | `cbrt` | 1 |
| 9–10 | combinations, permutations | 0 |
| 11 | trig (`sin`, `acos`, `sec`, …) | 11 |

Every puzzle has a solution. The hardest need trig, e.g.
`0000 = (acos(0) + acos(-0!) + acos(-0!)) / atan(0!)`.

## Rules

- Each digit is used exactly once, in order. No other numbers may appear.
- Joined digits can't start with 0 (`06` would just be `0 + 6`).
- Answers must be mathematically exact, not just close on a calculator. Undefined operations like
  `x / 0`, `x % 0` and `0^0` are never used.
- Within a tier, the cheapest solution wins (each operator has a weight), then the shortest.

## Build and run

You need [Rust](https://rustup.rs) and a C compiler:

- **Windows:** the rustup installer offers to install the Visual Studio C++ build tools; accept.
- **macOS:** `xcode-select --install`, then install rustup.
- **Linux:** `build-essential` (or your distro's `gcc`), then install rustup.

```bash
git clone https://github.com/PatrickL5420/traindle-explorer.git
cd traindle-explorer
cargo build --release
./target/release/traindle-explorer serve    # opens the explorer at http://127.0.0.1:7878/
```

On Windows, run `.\target\release\traindle-explorer.exe serve`. Run it from the repository folder;
saved runs go in `runs/`.

The default game takes about half a minute and ~1.5 GB of RAM.

## Using the explorer

- **Run:** pick the digit count and target, arrange the operators into tiers (drag and drop),
  then press Run. Presets offer some alternative rule sets.
- **Heatmap:** one cell per puzzle, coloured by tier. Hover for the solution, click for a tree
  diagram of it. You can also colour by cost, length or rarity (how few solutions a puzzle has).
- **Alternatives:** besides the best solution, up to four near-best ones are kept per puzzle.
- **Compare:** pick two runs (A and B) to see which puzzles got easier or harder, with a list of
  every change.
- **Sweep:** enter a range of targets (e.g. `1..20`) to run each one and compare them in a chart.
- **Verify:** every run is checked independently, both for mathematical exactness and that the
  game accepts each answer.

## Command line

```bash
./target/release/traindle-explorer                         # default game -> output/solutions.json
./target/release/traindle-explorer --digits 3 --target 24
./target/release/traindle-explorer --only 0000,0677 --stats
./target/release/traindle-explorer --help                  # all options
```

To check a results file independently (needs [Node.js](https://nodejs.org) 18+):

```bash
cd verify && npm install && cd ..
node verify/verify.mjs output/solutions.json
```

## How it works

- **Shared work:** the values reachable from a group of digits (say `47`) are the same in every
  puzzle containing them, so they're computed once and reused across all 10,000 puzzles.
- **Meet in the middle:** each puzzle is split in two (`24 | 79`), and the solver looks up the
  value one half needs from the other, instead of trying every pair.
- **Trig:** only angles that are rational multiples of π are kept. Common combinations like
  `sec(atan(x)) = √(1+x²)` count as single steps.
- **Exactness:** floating point can make a false expression look true (a tiny number rounding to
  0, for example). Every answer is re-checked at 256-bit precision before it's accepted.
- Everything runs in parallel across CPU cores (Rust + rayon).

For a ~15% faster build tuned to your own CPU, build with
`RUSTFLAGS="-C target-cpu=native" cargo build --release`. Don't share that binary: it may not run
on other machines.
