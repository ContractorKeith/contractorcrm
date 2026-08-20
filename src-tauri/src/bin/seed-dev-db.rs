//! Development-only seeder: builds a large, realistic ContractorCRM database
//! for scale testing (issue #42).
//!
//! Every record is written through the application seam, so the search index,
//! command log, and validation rules match a database a real user produced.
//!
//! Usage:
//!   cargo run --release --bin seed-dev-db -- --database /tmp/scale.sqlite3 --contacts 10000
//!
//! The file must not already exist — seeding is meant to produce a fresh
//! throwaway database, never to add load to somebody's real one.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use contractorcrm_lib::seed::{seed_database, SeedOptions};
use contractorcrm_lib::storage::Storage;

const USAGE: &str = "\
ContractorCRM development database seeder

Usage:
  seed-dev-db --database <path> [--contacts <n>] [--seed <n>] [--quiet]

Options:
  --database <path>  SQLite file to create. Must not already exist.
  --contacts <n>     Contacts to generate (default 10000). Everything else
                     scales off it: ~1 company per 5 contacts, ~1 opportunity
                     per 2, ~3 activities each, ~1 task per 2, plus tags and
                     custom fields on every tenth record.
  --seed <n>         RNG seed (default 42). The same seed produces the same
                     data every time.
  --quiet            No progress output, just the final summary.
  -h, --help         Show this message.

Notes:
  Development tool only — it opens the new database with
  `PRAGMA synchronous = NORMAL` so tens of thousands of seam transactions
  finish in minutes rather than hours. The app itself is untouched.
";

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_arguments(&arguments) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("seed-dev-db: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    if options.database.exists() {
        eprintln!(
            "seed-dev-db: {} already exists; point --database at a new file",
            options.database.display()
        );
        return ExitCode::FAILURE;
    }

    let mut storage = match Storage::open(&options.database) {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!("seed-dev-db: could not open the database: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Seeding writes one transaction per record through the seam; a full fsync
    // per commit is the whole runtime. Safe here — the file is disposable.
    if let Err(error) = storage
        .connection()
        .execute_batch("PRAGMA synchronous = NORMAL;")
    {
        eprintln!("seed-dev-db: could not set pragmas: {error}");
        return ExitCode::FAILURE;
    }

    let started = Instant::now();
    let quiet = options.quiet;
    let result = seed_database(
        &mut storage,
        &SeedOptions {
            contacts: options.contacts,
            seed: options.seed,
        },
        |phase, done, total| {
            if !quiet {
                eprintln!("  {phase}: {done}/{total}");
            }
        },
    );

    match result {
        Ok(summary) => {
            let seconds = started.elapsed().as_secs_f64();
            println!(
                "seeded {} in {seconds:.1}s: {} companies, {} contacts, {} opportunities, \
                 {} activities, {} tasks, {} tags, {} custom fields, {} records with metadata",
                options.database.display(),
                summary.companies,
                summary.contacts,
                summary.opportunities,
                summary.activities,
                summary.tasks,
                summary.tags,
                summary.custom_field_defs,
                summary.records_with_metadata,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("seed-dev-db: seeding failed: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    database: PathBuf,
    contacts: usize,
    seed: u64,
    quiet: bool,
}

/// Parse the command line. `Ok(None)` means "help was asked for".
fn parse_arguments(arguments: &[String]) -> Result<Option<Options>, String> {
    let defaults = SeedOptions::default();
    let mut database: Option<PathBuf> = None;
    let mut contacts = defaults.contacts;
    let mut seed = defaults.seed;
    let mut quiet = false;
    let mut index = 0;

    while index < arguments.len() {
        let argument = arguments[index].as_str();
        match argument {
            "-h" | "--help" => return Ok(None),
            "--quiet" => quiet = true,
            "--database" | "--contacts" | "--seed" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| format!("{argument} needs a value"))?;
                assign(argument, value, &mut database, &mut contacts, &mut seed)?;
            }
            other => {
                let (name, value) = other
                    .split_once('=')
                    .filter(|(name, _)| matches!(*name, "--database" | "--contacts" | "--seed"))
                    .ok_or_else(|| format!("unknown option: {other}"))?;
                assign(name, value, &mut database, &mut contacts, &mut seed)?;
            }
        }
        index += 1;
    }

    let database = database.ok_or_else(|| "--database <path> is required".to_owned())?;
    if contacts == 0 {
        return Err("--contacts must be at least 1".to_owned());
    }
    Ok(Some(Options {
        database,
        contacts,
        seed,
        quiet,
    }))
}

fn assign(
    name: &str,
    value: &str,
    database: &mut Option<PathBuf>,
    contacts: &mut usize,
    seed: &mut u64,
) -> Result<(), String> {
    match name {
        "--database" => *database = Some(PathBuf::from(value)),
        "--contacts" => {
            *contacts = value
                .parse()
                .map_err(|_| "--contacts needs a whole number".to_owned())?
        }
        "--seed" => {
            *seed = value
                .parse()
                .map_err(|_| "--seed needs a whole number".to_owned())?
        }
        _ => return Err(format!("unknown option: {name}")),
    }
    Ok(())
}
