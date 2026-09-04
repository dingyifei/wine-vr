//! Thin `--write` / `--check` shim over [`sabrage_contract_gen`].
//!
//! `--check` regenerates in memory and exits 1 when the committed
//! `scripts/demo/contract.gen.sh` differs; `--write` regenerates it in place.
//! `scripts/dev/parity.sh --regen` drives `--write`; the tier-1 parity test
//! drives the library directly.

use std::path::PathBuf;
use std::process::ExitCode;

use sabrage_contract_gen as gen;

const USAGE: &str = "\
usage: sabrage-contract-gen (--check | --write) [--repo-root <dir>]

  --check              regenerate in memory and diff against the committed
                       scripts/demo/contract.gen.sh; exit 1 on any mismatch
  --write              regenerate scripts/demo/contract.gen.sh in place
  --repo-root <dir>    wine-vr checkout to operate on (default: the checkout
                       this binary was built from)";

enum Mode {
    Check,
    Write,
}

fn main() -> ExitCode {
    let mut mode: Option<Mode> = None;
    let mut repo_root: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => mode = Some(Mode::Check),
            "--write" => mode = Some(Mode::Write),
            "--repo-root" => match args.next() {
                Some(v) => repo_root = Some(PathBuf::from(v)),
                None => return usage_error("--repo-root requires a value"),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return usage_error(&format!("unknown argument: {other}")),
        }
    }

    let root = repo_root.unwrap_or_else(gen::compiled_repo_root);
    let Some(mode) = mode else {
        return usage_error("one of --check or --write is required");
    };

    match mode {
        Mode::Check => match gen::check(&root) {
            Ok(report) if report.in_sync => {
                println!("contract.gen.sh in sync with contract/");
                ExitCode::SUCCESS
            }
            Ok(_) => {
                eprintln!(
                    "contract.gen.sh is out of sync with contract/ — regenerate with: \
                     scripts/dev/parity.sh --regen"
                );
                ExitCode::from(1)
            }
            Err(e) => fail(e),
        },
        Mode::Write => match gen::write(&root) {
            Ok(true) => {
                println!("wrote {}", gen::output_path(&root).display());
                ExitCode::SUCCESS
            }
            Ok(false) => {
                println!("unchanged: {}", gen::output_path(&root).display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
    }
}

fn fail(e: gen::GenError) -> ExitCode {
    eprintln!("sabrage-contract-gen: {e}");
    ExitCode::from(1)
}

/// Usage errors exit 2, matching demo.sh.
fn usage_error(message: &str) -> ExitCode {
    eprintln!("sabrage-contract-gen: {message}\n\n{USAGE}");
    ExitCode::from(2)
}
