//! `impose` — read a JobSpec (JSON or TOML), run the engine, write the imposed PDF.
//!
//! NOTE: the user-facing help/`about` strings below are provisional starter copy, written once at
//! the user's explicit request and expected to be revised. (Normally these would be copy-gap markers.)

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

/// Impose PDF pages onto press sheets.
#[derive(Parser)]
#[command(name = "impose")]
struct Cli {
    /// Path to the JobSpec file (.json or .toml).
    job: PathBuf,
    /// Path to write the imposed PDF.
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Diagnostic to stderr; conventional `error:` token, not composed copy.
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(&cli.job)?;
    let is_toml = cli
        .job
        .extension()
        .map(|e| e.eq_ignore_ascii_case("toml"))
        .unwrap_or(false);
    let job: vgdi_types::JobSpec = if is_toml {
        toml::from_str(&text)?
    } else {
        serde_json::from_str(&text)?
    };
    vgdi_engine::impose_to_file(&job, &cli.output)?;
    Ok(())
}
