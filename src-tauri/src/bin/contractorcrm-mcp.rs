//! ContractorCRM's local agent helper: an MCP server over stdio.
//!
//! Usage: `contractorcrm-mcp --database <path to contractorcrm.sqlite3> [--read-write]`
//!
//! Read-only unless `--read-write` is passed. No network listener is opened;
//! the agent client launches this process and talks to it over stdin/stdout.
//! All logic lives in `contractorcrm_lib::mcp` so it is testable without
//! spawning a process.

use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use contractorcrm_lib::mcp::{self, Mode, Server};

const USAGE: &str = "\
ContractorCRM agent helper (MCP over stdio)

Usage:
  contractorcrm-mcp --database <path> [--read-write]

Options:
  --database <path>  The app's SQLite file (see Settings → AI Assistant).
  --read-write       Allow write tools. Default is read-only.
  -h, --help         Show this message.
";

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_arguments(&arguments) {
        Ok(Some(options)) => options,
        // --help is a successful run that serves nothing.
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("contractorcrm-mcp: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let server = match Server::open(&options.database, options.mode) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("contractorcrm-mcp: {message}");
            return ExitCode::FAILURE;
        }
    };

    // Serve until the client closes stdin — that is the graceful shutdown.
    let stdin = BufReader::new(io::stdin());
    match mcp::serve(&server, stdin, io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("contractorcrm-mcp: stdio failed: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    database: PathBuf,
    mode: Mode,
}

/// Parse the command line. `Ok(None)` means "help was asked for".
fn parse_arguments(arguments: &[String]) -> Result<Option<Options>, String> {
    let mut database: Option<PathBuf> = None;
    let mut mode = Mode::ReadOnly;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "-h" | "--help" => return Ok(None),
            "--read-write" => mode = Mode::ReadWrite,
            "--database" => {
                index += 1;
                let path = arguments
                    .get(index)
                    .ok_or_else(|| "--database needs a path".to_owned())?;
                database = Some(PathBuf::from(path));
            }
            other => {
                if let Some(path) = other.strip_prefix("--database=") {
                    database = Some(PathBuf::from(path));
                } else {
                    return Err(format!("unknown option: {other}"));
                }
            }
        }
        index += 1;
    }

    let database = database.ok_or_else(|| "--database <path> is required".to_owned())?;
    Ok(Some(Options { database, mode }))
}
