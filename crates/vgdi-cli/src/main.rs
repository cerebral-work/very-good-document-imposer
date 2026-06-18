//! `impose` — read a JobSpec (JSON or TOML), run the engine, write the imposed PDF.
//!
//! ∑CG: user-facing CLI help/`about` prose and message wording are intentionally NOT authored
//! here (per the no-copywriting rule). Arg names are technical identifiers; the `about` strings
//! are left empty for the product owner to fill in. Grep `∑CG` for every copy gap.

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "impose")]
struct Cli {
    /// ∑CG: help text omitted
    job: PathBuf,
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
