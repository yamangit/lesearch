use anyhow::Result;
use clap::Parser;
use les_core::{Index, Query, QueryResult};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::{fs, io::Read, io::Write, sync::{Arc, Mutex}, thread};
use tokio::signal;

#[derive(Parser, Debug)]
#[command(name = "lesd", about = "Linux Everything-style search daemon")]
struct Args {
    /// Roots to index (default: /)
    #[arg(long, num_args = 1.., default_values_t = [String::from("/")])]
    roots: Vec<String>,

    /// Path to index database
    #[arg(long, default_value = "/var/lib/les/index.db")]
    db_path: String,

    /// Unix socket path for client communication
    #[arg(long, default_value = "/run/lesd.sock")]
    socket: String,

    /// Rebuild the index on start
    #[arg(long)]
    rebuild: bool,

    /// Exclude patterns (substring match)
    #[arg(long)]
    exclude: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Query { query: Query },
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Pong,
    QueryResult(QueryResult),
    Error { message: String },
}

fn handle_client(mut stream: UnixStream, index: Arc<Mutex<Index>>) -> Result<()> {
    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    let req: Request = match serde_json::from_str(&buf) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response::Error {
                message: format!("Invalid request: {e}"),
            };
            let _ = stream.write_all(serde_json::to_string(&resp)?.as_bytes());
            return Ok(());
        }
    };

    let resp = match req {
        Request::Ping => Response::Pong,
        Request::Query { query } => {
            let idx = index.lock().unwrap();
            match idx.run_query(&query) {
                Ok(r) => Response::QueryResult(r),
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
    };

    let out = serde_json::to_string(&resp)?;
    stream.write_all(out.as_bytes())?;
    Ok(())
}

fn start_fs_watcher(index: Arc<Mutex<Index>>, roots: Vec<String>, excludes: Vec<String>) -> Result<()> {
    let excludes_arc = Arc::new(excludes);

    thread::spawn(move || {
        let index_arc = index;
        let excludes_inner = excludes_arc;

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| match res {
                Ok(event) => {
                    if let Some(path) = event.paths.first() {
                        if matches!(
                            event.kind,
                            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                        ) {
                            let mut idx = index_arc.lock().unwrap();
                            idx.update_path(path, &excludes_inner);
                        }
                    }
                }
                Err(err) => {
                    eprintln!("watch error: {err}");
                }
            },
            Config::default(),
        )
        .expect("failed to create watcher");

        for r in &roots {
            if let Err(e) = watcher.watch(Path::new(r), RecursiveMode::Recursive) {
                eprintln!("Failed to watch {}: {e}", r);
            }
        }

        // Keep thread alive
        loop {
            std::thread::park();
        }
    });

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Ensure DB directory exists
    if let Some(parent) = Path::new(&args.db_path).parent() {
        fs::create_dir_all(parent)?;
    }

    let mut index = Index::open(Path::new(&args.db_path))?;

    if args.rebuild || index.entries.is_empty() {
        eprintln!("Building index from scratch...");
        index.rebuild(&args.roots, &args.exclude)?;
        eprintln!("Index built: {} entries", index.entries.len());
    } else {
        eprintln!(
            "Loaded existing index: {} entries from {}",
            index.entries.len(),
            args.db_path
        );
    }

    let shared_index = Arc::new(Mutex::new(index));

    // FS watcher (basic real-time updates)
    start_fs_watcher(shared_index.clone(), args.roots.clone(), args.exclude.clone())?;

    // Remove old socket if exists
    let socket_path = PathBuf::from(&args.socket);
    if socket_path.exists() {
        fs::remove_file(&socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    eprintln!("lesd listening on {}", args.socket);

    // Accept loop in a separate thread
    let index_for_accept = shared_index.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let idx = index_for_accept.clone();
                    thread::spawn(move || {
                        if let Err(e) = handle_client(stream, idx) {
                            eprintln!("client error: {e}");
                        }
                    });
                }
                Err(e) => {
                    eprintln!("accept error: {e}");
                }
            }
        }
    });

    // Wait for Ctrl+C (systemd will also send signals)
    signal::ctrl_c().await?;
    eprintln!("Shutting down lesd");

    // Remove socket on exit
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    Ok(())
}
