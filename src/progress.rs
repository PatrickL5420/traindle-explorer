//! Live progress + cancellation shared between the solver threads and the web server.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct Progress {
    /// puzzles in this run / solved so far
    pub total: AtomicUsize,
    pub solved: AtomicUsize,
    /// 1-based index of the current round / number of rounds
    pub round_idx: AtomicUsize,
    pub round_count: AtomicUsize,
    /// work items done / total in the current stage
    pub done: AtomicUsize,
    pub of: AtomicUsize,
    cancel: AtomicBool,
    text: Mutex<Text>,
}

#[derive(Default, Clone, Serialize)]
struct Text {
    round: String,
    stage: String,
    log: Vec<String>,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub total: usize,
    pub solved: usize,
    pub round_idx: usize,
    pub round_count: usize,
    pub round: String,
    pub stage: String,
    pub done: usize,
    pub of: usize,
    pub log: Vec<String>,
}

impl Progress {
    pub fn round(&self, idx: usize, label: &str) {
        self.round_idx.store(idx, Ordering::Relaxed);
        self.text.lock().unwrap().round = label.to_string();
    }
    pub fn stage(&self, name: &str, of: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.of.store(of, Ordering::Relaxed);
        self.text.lock().unwrap().stage = name.to_string();
    }
    pub fn log(&self, line: String) {
        let mut t = self.text.lock().unwrap();
        t.log.push(line);
        let n = t.log.len();
        if n > 200 {
            t.log.drain(..n - 200);
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    #[inline]
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn snapshot(&self) -> Snapshot {
        let t = self.text.lock().unwrap().clone();
        Snapshot {
            total: self.total.load(Ordering::Relaxed),
            solved: self.solved.load(Ordering::Relaxed),
            round_idx: self.round_idx.load(Ordering::Relaxed),
            round_count: self.round_count.load(Ordering::Relaxed),
            round: t.round,
            stage: t.stage,
            done: self.done.load(Ordering::Relaxed),
            of: self.of.load(Ordering::Relaxed),
            log: t.log,
        }
    }
}
