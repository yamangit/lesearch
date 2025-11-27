use anyhow::Result;
use chrono::{DateTime, Local};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sled::Db;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use walkdir::WalkDir;
use bincode; // <-- added

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64, // epoch seconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatternMode {
    Glob,
    Regex,
    Substr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub pattern: String,
    pub mode: PatternMode,
    pub files_only: bool,
    pub dirs_only: bool,
    pub roots: Vec<String>,
    pub exclude: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub min_mtime: Option<i64>,
    pub max_mtime: Option<i64>,
    /// Optional content pattern: if set, do a slower content search.
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub entries: Vec<FileEntry>,
}

/// Index abstraction: in-memory entries + sled DB on disk.
pub struct Index {
    pub entries: Vec<FileEntry>,
    db: Db,
}

impl Index {
    /// Open or create index DB and load entries into memory
    pub fn open(db_path: &Path) -> Result<Self> {
        let db = sled::open(db_path)?;
        let tree = db.open_tree("entries")?;
        let mut entries = Vec::new();

        for item in tree.iter() {
            let (_, v) = item?;
            let e: FileEntry = bincode::deserialize(&v)?;
            entries.push(e);
        }

        Ok(Self { entries, db })
    }

    /// Rebuild index from scratch for given roots
    pub fn rebuild(&mut self, roots: &[String], excludes: &[String]) -> Result<()> {
        let tree = self.db.open_tree("entries")?;
        tree.clear()?;
        self.entries.clear();

        for root in roots {
            self.index_root(Path::new(root), excludes)?;
        }

        // Persist entries into DB
        for entry in &self.entries {
            let key = entry.path.as_bytes();
            let val = bincode::serialize(entry)?;
            tree.insert(key, val)?;
        }
        tree.flush()?;
        Ok(())
    }

    fn index_root(&mut self, root: &Path, excludes: &[String]) -> Result<()> {
        for e in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|de| !should_skip(de.path(), excludes))
        {
            let e = match e {
                Ok(v) => v,
                Err(_) => continue,
            };

            let path = e.path();
            let md = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let is_dir = md.is_dir();
            let size = if is_dir { 0 } else { md.size() };
            let mtime = match md.modified() {
                Ok(t) => {
                    let dt: DateTime<Local> = t.into();
                    dt.timestamp()
                }
                Err(_) => 0,
            };

            let entry = FileEntry {
                path: path.to_string_lossy().to_string(),
                is_dir,
                size,
                mtime,
            };
            self.entries.push(entry);
        }
        Ok(())
    }

    /// Apply FS change: simple strategy – reindex that path or remove it
    pub fn update_path(&mut self, path: &Path, excludes: &[String]) {
        let s = path.to_string_lossy().to_string();

        // Remove any existing record for this path
        self.entries.retain(|e| e.path != s);

        if should_skip(path, excludes) {
            if let Ok(tree) = self.db.open_tree("entries") {
                let _ = tree.remove(s.as_bytes());
                let _ = tree.flush();
            }
            return;
        }

        if let Ok(md) = fs::metadata(path) {
            let is_dir = md.is_dir();
            let size = if is_dir { 0 } else { md.size() };
            let mtime = match md.modified() {
                Ok(t) => {
                    let dt: DateTime<Local> = t.into();
                    dt.timestamp()
                }
                Err(_) => 0,
            };

            let entry = FileEntry {
                path: s.clone(),
                is_dir,
                size,
                mtime,
            };

            // Serialize BEFORE pushing entry (fixes borrow-of-moved-value)
            if let Ok(tree) = self.db.open_tree("entries") {
                if let Ok(val) = bincode::serialize(&entry) {
                    let _ = tree.insert(s.as_bytes(), val);
                    let _ = tree.flush();
                }
            }

            // Now we can move entry
            self.entries.push(entry);

        } else {
            // path no longer exists -> remove from DB
            if let Ok(tree) = self.db.open_tree("entries") {
                let _ = tree.remove(s.as_bytes());
                let _ = tree.flush();
            }
        }
    }

    pub fn run_query(&self, q: &Query) -> Result<QueryResult> {
        let matcher = build_matcher(q)?;
        let mut out = Vec::new();

        'outer: for e in &self.entries {
            if q.files_only && e.is_dir {
                continue;
            }
            if q.dirs_only && !e.is_dir {
                continue;
            }

            if let Some(min) = q.min_size {
                if e.size < min {
                    continue;
                }
            }
            if let Some(max) = q.max_size {
                if e.size > max {
                    continue;
                }
            }
            if let Some(min) = q.min_mtime {
                if e.mtime < min {
                    continue;
                }
            }
            if let Some(max) = q.max_mtime {
                if e.mtime > max {
                    continue;
                }
            }

            // root filter
            if !q.roots.is_empty()
                && !q
                    .roots
                    .iter()
                    .any(|r| e.path.starts_with(r) || r == "/")
            {
                continue;
            }

            // exclude filter
            for ex in &q.exclude {
                if e.path.contains(ex) {
                    continue 'outer;
                }
            }

            if !matcher(&e.path) {
                continue;
            }

            // content search (slow, optional)
            if let Some(ref content_pattern) = q.content {
                if e.is_dir {
                    continue;
                }
                if !file_contains(&e.path, content_pattern) {
                    continue;
                }
            }

            out.push(e.clone());
        }

        Ok(QueryResult { entries: out })
    }
}

fn build_matcher(q: &Query) -> Result<Box<dyn Fn(&str) -> bool + Send + Sync>> {
    match q.mode {
        PatternMode::Glob => {
            let mut builder = GlobSetBuilder::new();
            builder.add(Glob::new(&q.pattern)?);
            let set: GlobSet = builder.build()?;
            Ok(Box::new(move |path: &str| {
                if let Some(fname) = Path::new(path).file_name().and_then(|s| s.to_str()) {
                    set.is_match(fname)
                } else {
                    false
                }
            }))
        }
        PatternMode::Regex => {
            let re = Regex::new(&q.pattern)?;
            Ok(Box::new(move |path: &str| re.is_match(path)))
        }
        PatternMode::Substr => {
            let needle = q.pattern.to_lowercase();
            Ok(Box::new(move |path: &str| {
                path.to_lowercase().contains(&needle)
            }))
        }
    }
}

fn file_contains(path: &str, needle: &str) -> bool {
    // Simple, non-indexed content search (slow but optional)
    if let Ok(text) = fs::read_to_string(path) {
        text.contains(needle)
    } else {
        false
    }
}

fn should_skip(path: &Path, excludes: &[String]) -> bool {
    const DEFAULT_SKIP: &[&str] = &[
        "/proc", "/sys", "/dev", "/run", "/tmp", "/var/run", "/var/tmp", "/var/cache",
        "/var/lib/snapd",
    ];
    let s = path.to_string_lossy();

    if DEFAULT_SKIP.iter().any(|p| s.starts_with(p)) {
        return true;
    }

    for ex in excludes {
        if s.contains(ex) {
            return true;
        }
    }
    false
}

