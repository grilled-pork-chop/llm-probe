//! `llmprobe` binary entry point.

use clap::Parser;
use llmprobe::cli::Args;
use llmprobe::config::RunConfig;
use llmprobe::metrics::aggregate;
use llmprobe::report;
use llmprobe::runner::run_grow;
use std::io::IsTerminal;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let json = args.json;
    let color = !json && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();

    let cfg = match RunConfig::build(
        &args.url,
        args.model,
        args.conversations,
        args.concurrency,
        args.stream,
        args.system,
        args.max_turns,
        args.max_tokens,
        args.seed,
        args.timeout,
        args.api_key,
        &args.headers,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    let use_tui = !args.no_tui && !json && cfg!(feature = "tui");

    let result = if use_tui {
        #[cfg(feature = "tui")]
        {
            match llmprobe::tui::run(&cfg).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        #[cfg(not(feature = "tui"))]
        {
            unreachable!("tui disabled at compile time")
        }
    } else {
        match run_grow(&cfg, None, None).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        }
    };

    let summary = aggregate(&result);

    if json {
        match report::render_json(&result) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: failed to serialize report: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        print!("{}", report::render(&result, color));
    }

    exit_code(&summary)
}

fn exit_code(report: &llmprobe::metrics::RunReport) -> ExitCode {
    if report.conversations.total > 0 && report.conversations.errors == report.conversations.total {
        return ExitCode::from(1);
    }
    if report.conversations.errors > 0 {
        return ExitCode::from(2);
    }
    ExitCode::from(0)
}
