//! Thin entry point: parse arguments, set up logging, delegate to
//! [`gerenuk::cli::Cli::run`], and map its outcome onto an exit code.

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use gerenuk::cli::{Cli, Outcome};

/// Exit code for a run that could not complete (missing `tyf`, bad workspace).
const EXIT_ERROR: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    let result = cli.run(&mut out).and_then(|outcome| {
        out.flush()?;
        Ok(outcome)
    });

    match result {
        Ok(Outcome::Clean) => ExitCode::SUCCESS,
        Ok(Outcome::FindingsReported) => ExitCode::FAILURE,
        // pytest's own code, propagated verbatim — that is the hook contract.
        // Only reachable where the process could not be replaced outright.
        Ok(Outcome::Code(code)) => ExitCode::from(code),
        Err(err) => {
            eprintln!("gerenuk: {err:#}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// `RUST_LOG` wins when set; `--verbose` only raises the default level.
fn init_tracing(verbose: bool) {
    let default = if verbose { "debug" } else { "warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(io::stderr).init();
}
