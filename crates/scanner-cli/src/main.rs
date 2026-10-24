//! `scanner` — scans a folder or drive and reports used disk space.

mod output;

use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use scanner_core::{CancelToken, Progress, display_path, format_size, scan};

use output::Filter;

#[derive(Parser)]
#[command(name = "scanner", version, about = "Scan a folder or drive and report used disk space")]
struct Args {
    /// Folder or drive to scan, e.g. `C:\` or `D:\proj`.
    path: PathBuf,

    #[arg(short, long, value_enum, default_value_t = Format::Summary)]
    format: Format,

    /// Deepest level to output below the root (json, csv). Unlimited if omitted.
    #[arg(short, long)]
    depth: Option<usize>,

    /// Number of largest entries to list (summary).
    #[arg(short = 'n', long, default_value_t = 20)]
    top: usize,

    /// Leave out entries taking less than this many bytes (json, csv).
    #[arg(long, default_value_t = 0)]
    min_size: u64,

    /// Write to this file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Do not show progress on stderr.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Human-readable list of the largest entries.
    Summary,
    /// Nested tree with stats.
    Json,
    /// One row per entry.
    Csv,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("scanner: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> io::Result<()> {
    let cancel = CancelToken::new();
    {
        let cancel = cancel.clone();
        // Ctrl+C stops the walk; whatever was read so far is still reported.
        ctrlc::set_handler(move || cancel.cancel()).map_err(io::Error::other)?;
    }

    let show_progress = !args.quiet && io::stderr().is_terminal();
    let result = scan(&args.path, &cancel, |p| {
        if show_progress {
            print_progress(p);
        }
    })?;
    if show_progress {
        eprint!("\r\x1b[2K");
    }
    if result.stats.cancelled {
        eprintln!("scanner: cancelled, results are partial");
    }

    let filter = Filter {
        max_depth: args.depth.unwrap_or(usize::MAX),
        min_size: args.min_size,
    };
    let mut out: Box<dyn Write> = match &args.output {
        Some(path) => Box::new(BufWriter::new(File::create(path)?)),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };
    match args.format {
        Format::Summary => output::write_summary(&mut out, &result, args.top)?,
        Format::Json => output::write_json(&mut out, &result, filter)?,
        Format::Csv => output::write_csv(&mut out, &result.tree, filter)?,
    }
    out.flush()
}

fn print_progress(p: &Progress) {
    const MAX_PATH_CHARS: usize = 60;
    let dir = output::printable(&display_path(p.current_dir).to_string_lossy()).into_owned();
    let chars: Vec<char> = dir.chars().collect();
    let dir = if chars.len() > MAX_PATH_CHARS {
        let tail: String = chars[chars.len() - (MAX_PATH_CHARS - 1)..].iter().collect();
        format!("…{tail}")
    } else {
        dir
    };
    eprint!(
        "\r\x1b[2K{} files, {} folders, {}  {dir}",
        p.files,
        p.dirs,
        format_size(p.allocated)
    );
}
