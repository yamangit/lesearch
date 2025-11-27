use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use clap::{Parser, ValueEnum};
use les_core::{PatternMode, Query};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[derive(Parser, Debug)]
#[command(name = "les", about = "Linux Everything-style search client")]
struct Args {
    /// Query pattern (file name/path)
    pattern: Option<String>,

    /// Pattern mode
    ///#[arg(long, value_enum, default_value_t = Mode::Substr)]
    #[arg(long, value_enum, default_value_t = Mode::Substr)]
    mode: Mode,

    /// Only files
    #[arg(long)]
    files_only: bool,

    /// Only directories
    #[arg(long)]
    dirs_only: bool,

    /// Roots to search (must be subset of daemon roots)
    #[arg(long, num_args = 1..)]
    roots: Vec<String>,

    /// Exclude substring filters
    #[arg(long)]
    exclude: Vec<String>,

    /// Minimum size in bytes
    #[arg(long)]
    min_size: Option<u64>,

    /// Maximum size in bytes
    #[arg(long)]
    max_size: Option<u64>,

    /// Minimum modification time (UNIX epoch seconds)
    #[arg(long)]
    min_mtime: Option<i64>,

    /// Maximum modification time (UNIX epoch seconds)
    #[arg(long)]
    max_mtime: Option<i64>,

    /// Content search string (slow)
    #[arg(long)]
    content: Option<String>,

    /// Unix socket path (must match lesd)
    #[arg(long, default_value = "/run/lesd.sock")]
    socket: String,

    /// Interactive mode: repeatedly prompt for pattern
    #[arg(short, long)]
    interactive: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Mode {
    Glob,
    Regex,
    Substr,
}

impl From<Mode> for PatternMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Glob => PatternMode::Glob,
            Mode::Regex => PatternMode::Regex,
            Mode::Substr => PatternMode::Substr,
        }
    }
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
    QueryResult(les_core::QueryResult),
    Error { message: String },
}

fn send_request(socket: &str, req: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket)?;
    let data = serde_json::to_string(req)?;
    stream.write_all(data.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    let resp: Response = serde_json::from_str(&buf)?;
    Ok(resp)
}

fn print_results(resp: Response) {
    match resp {
        Response::Pong => println!("OK (pong)"),
        Response::Error { message } => eprintln!("Error: {message}"),
        Response::QueryResult(qr) => {
            for e in qr.entries {
                let dt = DateTime::<Utc>::from_timestamp(e.mtime, 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap())
                    .with_timezone(&Local);
                println!(
                    "{}\t{}\t{}\t{}",
                    if e.is_dir { "d" } else { "-" },
                    e.size,
                    dt.format("%Y-%m-%d %H:%M:%S"),
                    e.path
                );
            }
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.files_only && args.dirs_only {
        eprintln!("--files-only and --dirs-only cannot both be set");
        std::process::exit(1);
    }

    if args.interactive {
        // minimal TUI: read pattern, query, print results
        use std::io::{self, Write};
        loop {
            print!("les> ");
            io::stdout().flush().unwrap();

            let mut p = String::new();
            if io::stdin().read_line(&mut p).is_err() {
                break;
            }
            let p = p.trim().to_string();
            if p.is_empty() {
                break;
            }

            let q = Query {
                pattern: p,
                mode: args.mode.into(),
                files_only: args.files_only,
                dirs_only: args.dirs_only,
                roots: args.roots.clone(),
                exclude: args.exclude.clone(),
                min_size: args.min_size,
                max_size: args.max_size,
                min_mtime: args.min_mtime,
                max_mtime: args.max_mtime,
                content: args.content.clone(),
            };
            let req = Request::Query { query: q };
            match send_request(&args.socket, &req) {
                Ok(resp) => print_results(resp),
                Err(e) => eprintln!("Error: {e}"),
            }
        }

        Ok(())
    } else {
        let pattern = args
            .pattern
            .unwrap_or_else(|| {
                eprintln!("Pattern is required in non-interactive mode");
                std::process::exit(1);
            });

        let q = Query {
            pattern,
            mode: args.mode.into(),
            files_only: args.files_only,
            dirs_only: args.dirs_only,
            roots: args.roots.clone(),
            exclude: args.exclude.clone(),
            min_size: args.min_size,
            max_size: args.max_size,
            min_mtime: args.min_mtime,
            max_mtime: args.max_mtime,
            content: args.content.clone(),
        };

        let req = Request::Query { query: q };
        let resp = send_request(&args.socket, &req)?;
        print_results(resp);
        Ok(())
    }
}
