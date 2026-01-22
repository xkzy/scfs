use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FreeRun {
    start: u64,
    len: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct FreeExtentOnDisk {
    runs: Vec<FreeRun>,
}

/// In-memory free-extent index with persistence
#[derive(Clone, Debug)]
pub struct FreeExtentIndex {
    /// Map start -> len
    runs: BTreeMap<u64, u64>,
    /// Map len -> set of starts (for best-fit search)
    by_len: BTreeMap<u64, BTreeSet<u64>>,
    persist_path: Option<PathBuf>,
}

impl FreeExtentIndex {
    pub fn new(persist_path: Option<PathBuf>) -> Result<Self> {
        let mut idx = FreeExtentIndex {
            runs: BTreeMap::new(),
            by_len: BTreeMap::new(),
            persist_path: persist_path.clone(),
        };
        if let Some(path) = &persist_path {
            if path.exists() {
                idx.load(path)?;
            }
        }
        Ok(idx)
    }

    fn load(&mut self, path: &PathBuf) -> Result<()> {
        let contents = fs::read(path).context("Failed to read free-extent index file")?;
        let ond: FreeExtentOnDisk = bincode::deserialize(&contents)
            .context("Failed to deserialize free-extent index")?;
        for r in ond.runs {
            self.insert_run_internal(r.start, r.len);
        }
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        if let Some(path) = &self.persist_path {
            let ond = FreeExtentOnDisk {
                runs: self.runs.iter().map(|(s,l)| FreeRun{start:*s,len:*l}).collect()
            };
            let tmp = path.with_extension("fe.tmp");
            let encoded = bincode::serialize(&ond).context("Failed to serialize free-extent index")?;
            fs::write(&tmp, encoded).context("Failed to write tmp free-extent index")?;
            fs::rename(&tmp, path).context("Failed to atomically persist free-extent index")?;
        }
        Ok(())
    }

    /// Insert a free run, merging with adjacent runs where possible
    pub fn insert_run(&mut self, start: u64, len: u64) -> Result<()> {
        self.insert_run_internal(start, len);
        self.persist()?;
        Ok(())
    }

    fn insert_run_internal(&mut self, mut start: u64, mut len: u64) {
        // Merge with previous run if contiguous
        if let Some((&prev_start, &prev_len)) = self.runs.range(..start).next_back() {
            if prev_start + prev_len == start {
                // remove prev
                self.remove_run(prev_start);
                start = prev_start;
                len = len + prev_len;
            }
        }
        // Merge with next run if contiguous
        if let Some((&next_start, &next_len)) = self.runs.range(start..).next() {
            if start + len == next_start {
                self.remove_run(next_start);
                len = len + next_len;
            }
        }
        // Insert merged
        self.runs.insert(start, len);
        self.by_len.entry(len).or_default().insert(start);
    }

    fn remove_run(&mut self, start: u64) {
        if let Some(len) = self.runs.remove(&start) {
            if let Some(set) = self.by_len.get_mut(&len) {
                set.remove(&start);
                if set.is_empty() {
                    self.by_len.remove(&len);
                }
            }
        }
    }

    /// Find a best-fit run (smallest len >= n) and return its start
    pub fn allocate_best_fit(&mut self, n: u64) -> Option<u64> {
        // find first key >= n
        let key_opt = self.by_len.range(n..).next().map(|(k, _)| *k);
        if key_opt.is_none() {
            return None;
        }
        let key = key_opt.unwrap();
        if let Some(set) = self.by_len.get_mut(&key) {
            if let Some(&start) = set.iter().next() {
                // consume from this run
                set.remove(&start);
                if set.is_empty() {
                    self.by_len.remove(&key);
                }
                // remove run
                self.runs.remove(&start);
                if key > n {
                    // split remainder and insert back
                    let rem_start = start + n;
                    let rem_len = key - n;
                    self.insert_run_internal(rem_start, rem_len);
                }
                // persist
                let _ = self.persist();
                return Some(start);
            }
        }
        None
    }

    /// Consume a specific range (assumes range is within a run) - used when bitmap allocates
    pub fn consume_range(&mut self, start: u64, len: u64) -> Result<()> {
        // find run that contains [start, start+len)
        let (&run_start, &run_len) = self.runs.range(..=start).next_back().ok_or_else(|| anyhow!("No run contains start"))?;
        if start < run_start || start + len > run_start + run_len {
            return Err(anyhow!("Requested range not fully contained in run"));
        }
        // remove original
        self.remove_run(run_start);
        // insert before fragment
        if start > run_start {
            self.insert_run_internal(run_start, start - run_start);
        }
        // insert after fragment
        let after = start + len;
        let run_end = run_start + run_len;
        if after < run_end {
            self.insert_run_internal(after, run_end - after);
        }
        self.persist()?;
        Ok(())
    }

    /// For debugging: list runs
    pub fn list_runs(&self) -> Vec<(u64,u64)> {
        self.runs.iter().map(|(s,l)| (*s,*l)).collect()
    }
}
