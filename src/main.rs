use anyhow::Result;
use clap::{ArgGroup, Parser};
use globset::{Glob, GlobSetBuilder};
use rayon::prelude::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Lightning-fast filename search for Linux (Everything-style prototype)
#[derive(Parser, Debug)]
#[command(name = "les", version, about = "Linux Everything-style file search")]
#[command(group(
    ArgGroup::new("pattern_type")
        .required(true)
        .args(["glob", "regex", "substr"])
))]
struct Args {
    /// Root directory to index (default: /)
    #[arg(short, long, default_value = "/")]
    root: String,

    /// Simple glob pattern, e.g. *.log or *report*2024*
    #[arg(long)]
    glob: Option<String>,

    /// Regex pattern, e.g. (?i)error_\\d+\\.log
    #[arg(long)]
    regex: Option<String>,

    /// Simple substring match (case-insensitive)
    #[arg(long)]
    substr: Option<String>,

    /// Only files
    #[arg(long)]
    files_only: bool,

    /// Only directories
    #[arg(long)]
    dirs_only: bool,

    /// Exclude paths containing this substring (can be used multiple times)
    #[arg(long)]
    exclude: Vec<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    is_dir: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.files_only && args.dirs_only {
        eprintln!("--files-only and --dirs-only are mutually exclusive");
        std::process::exit(1);
    }

    let root = Path::new(&args.root);
    if !root.exists() {
        eprintln!("Root path does not exist: {}", root.display());
        std::process::exit(1);
    }

    // 1) Build index
    eprintln!("Indexing {} ... (this may take a bit on first run)", root.display());
    let entries = build_index(root, &args.exclude)?;
    eprintln!("Indexed {} entries", entries.len());

    // 2) Build matcher
    let matcher = build_matcher(&args)?;

    // 3) Filter in parallel for speed
    entries
        .par_iter()
        .filter(|e| {
            if args.files_only && e.is_dir {
                return false;
            }
            if args.dirs_only && !e.is_dir {
                return false;
            }
            matcher(&e.path)
        })
        .for_each(|e| {
            println!("{}", e.path.display());
        });

    Ok(())
}

fn build_index(root: &Path, excludes: &[String]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !should_skip(e.path(), excludes))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // skip unreadable entries
        };

        let path = entry.path().to_path_buf();
        let is_dir = entry.file_type().is_dir();

        out.push(Entry { path, is_dir });
    }

    Ok(out)
}

fn should_skip(path: &Path, excludes: &[String]) -> bool {
    // Always skip some common virtual/temporary dirs
    const DEFAULT_SKIP: &[&str] = &[
        "/proc",
        "/sys",
        "/dev",
        "/run",
        "/tmp",
        "/var/run",
        "/var/tmp",
        "/var/cache",
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

fn build_matcher(args: &Args) -> Result<Box<dyn Fn(&Path) -> bool + Sync + Send>> {
    if let Some(g) = &args.glob {
        // Glob matcher
        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new(g)?);
        let set = builder.build()?;

        Ok(Box::new(move |path: &Path| {
            let fname = match path.file_name().and_then(|s| s.to_str()) {
                Some(f) => f,
                None => return false,
            };
            set.is_match(fname)
        }))
    } else if let Some(r) = &args.regex {
        let re = Regex::new(r)?;
        Ok(Box::new(move |path: &Path| {
            let fname = match path.to_str() {
                Some(f) => f,
                None => return false,
            };
            re.is_match(fname)
        }))
    } else if let Some(s) = &args.substr {
        let needle = s.to_lowercase();
        Ok(Box::new(move |path: &Path| {
            let fname = match path.to_str() {
                Some(f) => f,
                None => return false,
            };
            fname.to_lowercase().contains(&needle)
        }))
    } else {
        unreachable!("clap ensures one of glob/regex/substr is set");
    }
}

