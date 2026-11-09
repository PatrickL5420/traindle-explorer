//! On-disk layout of a solved run (a `runs/<id>/` folder, or the CLI's `output/`).
//!
//! * `config.json`, `summary.json`: the settings and the stats.
//! * `grid.bin`: tier, length, cost and rarity count of every puzzle (see `grid_bytes`).
//! * `solutions.txt`: one line per puzzle in puzzle order (line i = puzzle i), holding
//!   its best expression, or empty if unsolved.
//! * `alts.bin`: the alternatives, zlib-compressed in blocks of `BLOCK` puzzles so one
//!   puzzle can be read without unpacking the rest. Layout (little-endian): `b"TXA1"`,
//!   u32 puzzle count, u32 block size, u32 block count, then per block (u64 offset,
//!   u32 length), then the blocks. A block is a JSON array with one entry per puzzle:
//!   `[]` or `[[cost, "expr"], ...]` (cost is null for runs that didn't record it).
//!
//! The game-format `solutions.json` (`[{number, solution, tier, cost, count}]`) is
//! generated on demand by `game_json` rather than stored.

use crate::runs::{grid_bytes, write_json, Summary};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const BLOCK: usize = 1000;

/// One solved puzzle, as stored.
pub struct Stored<'a> {
    pub index: usize,
    pub solution: &'a str,
    pub tier: u8,
    pub cost: Option<u32>,
    pub count: Option<u16>,
    /// (cost, expression); cost None for runs saved before costs were recorded
    pub alts: Vec<(Option<u32>, &'a str)>,
}

/// Write every data file of a run into `dir`.
pub fn write_run(dir: &Path, summary: &Summary, digits: usize, puzzles: &[Stored]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_json(&dir.join("config.json"), &summary.config, true)?;
    write_data(dir, digits, puzzles)?;
    write_json(&dir.join("summary.json"), summary, true)
}

/// `grid.bin`, `solutions.txt` and `alts.bin`.
pub fn write_data(dir: &Path, digits: usize, puzzles: &[Stored]) -> std::io::Result<()> {
    let n = 10usize.pow(digits as u32);
    let mut by: Vec<Option<&Stored>> = vec![None; n];
    for p in puzzles {
        if p.index < n {
            by[p.index] = Some(p);
        }
    }
    std::fs::write(
        dir.join("grid.bin"),
        grid_bytes(digits, puzzles.iter().map(|p| (p.index, p.tier, p.solution.len(), p.cost, p.count))),
    )?;

    let mut txt = std::io::BufWriter::new(std::fs::File::create(dir.join("solutions.txt"))?);
    for p in &by {
        writeln!(txt, "{}", p.map_or("", |p| p.solution))?;
    }
    txt.flush()?;

    let nblocks = n.div_ceil(BLOCK);
    let mut blocks: Vec<Vec<u8>> = Vec::with_capacity(nblocks);
    for b in 0..nblocks {
        let entries: Vec<&[(Option<u32>, &str)]> =
            by[b * BLOCK..((b + 1) * BLOCK).min(n)].iter().map(|p| p.map_or(&[][..], |p| &p.alts[..])).collect();
        let mut z = ZlibEncoder::new(Vec::new(), Compression::new(6));
        z.write_all(serde_json::to_string(&entries)?.as_bytes())?;
        blocks.push(z.finish()?);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"TXA1");
    for v in [n as u32, BLOCK as u32, nblocks as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut offset = (16 + 12 * nblocks) as u64;
    for b in &blocks {
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        offset += b.len() as u64;
    }
    for b in &blocks {
        out.extend_from_slice(b);
    }
    std::fs::write(dir.join("alts.bin"), out)
}

/// Every puzzle's best expression (None = unsolved), from `solutions.txt`.
pub fn read_texts(dir: &Path) -> std::io::Result<Vec<Option<String>>> {
    let text = std::fs::read_to_string(dir.join("solutions.txt"))?;
    Ok(text.lines().map(|l| if l.is_empty() { None } else { Some(l.to_string()) }).collect())
}

/// The alternatives of puzzle `index` as JSON (`[[cost, "expr"], ...]`, `[]` if none).
pub fn read_alts(dir: &Path, index: usize) -> std::io::Result<String> {
    let mut f = std::fs::File::open(dir.join("alts.bin"))?;
    let mut head = [0u8; 16];
    f.read_exact(&mut head)?;
    if &head[..4] != b"TXA1" {
        return Err(std::io::Error::other("not an alternatives file"));
    }
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize;
    let (n, block, nblocks) = (u32_at(&head, 4), u32_at(&head, 8), u32_at(&head, 12));
    if index >= n {
        return Ok("[]".into());
    }
    let b = index / block;
    if b >= nblocks {
        return Ok("[]".into());
    }
    let mut entry = [0u8; 12];
    f.seek(SeekFrom::Start((16 + 12 * b) as u64))?;
    f.read_exact(&mut entry)?;
    let offset = u64::from_le_bytes(entry[..8].try_into().unwrap());
    let len = u32_at(&entry, 8);
    let mut packed = vec![0u8; len];
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(&mut packed)?;
    let mut json = String::new();
    ZlibDecoder::new(&packed[..]).read_to_string(&mut json)?;
    let v: Value = serde_json::from_str(&json).map_err(std::io::Error::other)?;
    Ok(v.get(index % block).map_or("[]".into(), |e| e.to_string()))
}

/// The game-format solutions file, built from `solutions.txt` and `grid.bin`.
pub fn game_json(dir: &Path, digits: usize) -> std::io::Result<String> {
    let texts = read_texts(dir)?;
    let grid = std::fs::read(dir.join("grid.bin"))?;
    let n = u32::from_le_bytes(grid[4..8].try_into().unwrap()) as usize;
    let recorded = grid[8] & 1 == 1;
    let tier = &grid[16..16 + n];
    let u16_at = |base: usize, i: usize| u16::from_le_bytes([grid[base + 2 * i], grid[base + 2 * i + 1]]);
    let mut out = String::with_capacity(texts.len() * 80);
    out.push('[');
    let mut first = true;
    for (i, t) in texts.iter().enumerate() {
        let Some(t) = t else { continue };
        if i >= n {
            break;
        }
        if !first {
            out.push(',');
        }
        first = false;
        // written by hand to keep the game format's key order (serde_json would sort them)
        out.push_str(&format!(r#"{{"number":"{i:0digits$}","solution":{},"tier":{}"#, serde_json::to_string(t)?, tier[i]));
        if recorded {
            out.push_str(&format!(r#","cost":{},"count":{}"#, u16_at(16 + 2 * n, i), u16_at(16 + 4 * n, i)));
        }
        out.push('}');
    }
    out.push(']');
    Ok(out)
}

/// Convert a run saved in an older layout (`solutions.json`, optionally with inline
/// `alts` or a `solutions.alts.jsonl` file) to the current one. Returns true if it did.
pub fn upgrade(dir: &Path) -> std::io::Result<bool> {
    let old = dir.join("solutions.json");
    if dir.join("solutions.txt").exists() || !old.exists() {
        return Ok(false);
    }
    #[derive(serde::Deserialize)]
    struct Old {
        number: String,
        solution: String,
        tier: u8,
        cost: Option<u32>,
        count: Option<u16>,
        #[serde(default)]
        alts: Vec<String>,
    }
    let summary: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("summary.json"))?).map_err(std::io::Error::other)?;
    let digits = summary["digits"].as_u64().unwrap_or(4) as usize;
    let entries: Vec<Old> = serde_json::from_str(&std::fs::read_to_string(&old)?).map_err(std::io::Error::other)?;
    // alternatives kept in a separate JSON-lines file: ["0123", [[cost, "expr"], ...]]
    let alt_file = dir.join("solutions.alts.jsonl");
    let mut extra: std::collections::HashMap<String, Vec<(Option<u32>, String)>> = Default::default();
    if let Ok(text) = std::fs::read_to_string(&alt_file) {
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (num, alts): (String, Vec<(Option<u32>, String)>) = serde_json::from_str(line).map_err(std::io::Error::other)?;
            extra.insert(num, alts);
        }
    }
    let puzzles: Vec<Stored> = entries
        .iter()
        .map(|e| Stored {
            index: e.number.parse().unwrap_or(usize::MAX),
            solution: &e.solution,
            tier: e.tier,
            cost: e.cost,
            count: e.count,
            alts: match extra.get(&e.number) {
                Some(a) => a.iter().map(|(c, s)| (*c, s.as_str())).collect(),
                None => e.alts.iter().map(|s| (None, s.as_str())).collect(),
            },
        })
        .collect();
    write_data(dir, digits, &puzzles)?;
    std::fs::remove_file(&old)?;
    let _ = std::fs::remove_file(&alt_file);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("txstore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let puzzles = vec![
            Stored { index: 7, solution: "2 * 5", tier: 1, cost: Some(20), count: Some(2), alts: vec![(Some(21), "10 / 1")] },
            Stored { index: 2500, solution: "(9 + 1)", tier: 2, cost: Some(30), count: Some(1), alts: vec![] },
        ];
        write_data(&dir, 4, &puzzles).unwrap();
        let texts = read_texts(&dir).unwrap();
        assert_eq!(texts.len(), 10000);
        assert_eq!(texts[7].as_deref(), Some("2 * 5"));
        assert_eq!(texts[8], None);
        assert_eq!(read_alts(&dir, 7).unwrap(), r#"[[21,"10 / 1"]]"#);
        assert_eq!(read_alts(&dir, 2500).unwrap(), "[]");
        let game: Value = serde_json::from_str(&game_json(&dir, 4).unwrap()).unwrap();
        assert_eq!(game[0]["number"], "0007");
        assert_eq!(game[1]["tier"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
