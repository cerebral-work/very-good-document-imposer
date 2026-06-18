//! `impose` — read a JobSpec (JSON or TOML), run the engine, write the imposed PDF.
//!
//! User-facing CLI help/`about`/message wording is intentionally NOT authored here (no-copywriting
//! rule). Each gap is marked `∑CG` with a commented-out spec + sample to seed the final wording;
//! arg names are technical identifiers. Grep `∑CG` for every copy gap.

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

// ∑CG: command description shown in `impose --help`
//   spec: one line, ≤ ~60 chars, states what the tool does
//   sample: "Impose PDF pages onto press sheets."
// (Deliberately NOT a `///` doc comment on the struct — that would render as the about text.)
#[derive(Parser)]
#[command(name = "impose")]
struct Cli {
    // ∑CG: positional-arg help for the input JobSpec path
    //   spec: short noun phrase, ≤ ~50 chars
    //   sample: "Path to the JobSpec file (.json or .toml)"
    // (`//` not `///`, so clap shows no help — avoids leaking the token to users.)
    job: PathBuf,
    // ∑CG: help for the --output flag
    //   spec: short phrase, ≤ ~50 chars
    //   sample: "Path to write the imposed PDF"
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
