//! `llmprobe` binary entry point: parse CLI → `RunConfig` → run → exit code.

use clap::Parser;
use llmprobe::cli::Args;
use llmprobe::config::RunConfig;
use llmprobe::metrics::{Report, aggregate};
use llmprobe::report;
use llmprobe::runner::{BatchResult, run_batch};
use std::io::IsTerminal;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // A.13: `--tui` always parses; reject it on a build without the feature.
    #[cfg(not(feature = "tui"))]
    if args.tui {
        eprintln!("error: this build has no TUI support; rebuild with --features tui");
        return ExitCode::from(1);
    }

    let json = args.json;
    // Color: off for JSON, off unless stdout is a TTY, honored by --no-color and
    // the NO_COLOR convention (A.12).
    let color = !json
        && !args.no_color
        && std::env::var_os("NO_COLOR").is_none()
        && std::io::stdout().is_terminal();

    let cfg = match RunConfig::build(
        &args.url,
        args.model,
        args.requests,
        args.concurrency,
        args.stream,
        args.prompt,
        args.max_tokens,
        args.temperature,
        args.timeout,
        args.warmup,
        args.api_key,
        &args.headers,
    ) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    // TUI path (feature-gated): the dashboard runs the batch and returns the
    // same BatchResult, then we fall through to the shared report + exit code.
    let batch_result = if args.tui {
        #[cfg(feature = "tui")]
        {
            llmprobe::tui::run(&cfg).await
        }
        #[cfg(not(feature = "tui"))]
        {
            unreachable!("--tui rejected earlier on no-feature builds")
        }
    } else {
        run_batch(&cfg, None, None).await
    };

    let BatchResult {
        outcomes,
        wall_clock,
    } = match batch_result {
        Ok(res) => res,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    let summary = aggregate(&outcomes, wall_clock);

    if json {
        match report::render_json(&summary, &cfg) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: failed to serialize report: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        print!("{}", report::render(&summary, &cfg, color));
    }

    exit_code(&summary)
}

/// Exit code per §8: `0` all ok, `1` total failure, `2` partial failure.
fn exit_code(report: &Report) -> ExitCode {
    if report.total > 0 && report.ok == 0 {
        return ExitCode::from(1);
    }
    if report.failed > 0 {
        return ExitCode::from(2);
    }
    ExitCode::from(0)
}
