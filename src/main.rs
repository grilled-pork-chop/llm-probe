//! `llmprobe` binary entry point.

use clap::Parser;
use llmprobe::cli::Args;
use llmprobe::config::RunConfig;
use llmprobe::metrics::aggregate;
use llmprobe::persist;
use llmprobe::report;
use llmprobe::runner::run_grow;
use std::io::IsTerminal;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let json = args.json;
    let color = !json
        && !args.no_color
        && std::env::var_os("NO_COLOR").is_none()
        && std::io::stdout().is_terminal();

    // ── Replay mode: load file and display without running anything ──────────
    if let Some(ref path) = args.replay {
        let result = match persist::load(path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };

        #[cfg(feature = "tui")]
        if args.tui {
            match llmprobe::tui::replay(&result).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            }
            if json {
                match report::render_json(&result) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                print!("{}", report::render(&result, color));
            }
            return ExitCode::from(0);
        }

        if json {
            match report::render_json(&result) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            }
        } else {
            print!("{}", report::render(&result, color));
        }
        return ExitCode::from(0);
    }

    // ── Live run ─────────────────────────────────────────────────────────────
    let url = match &args.url {
        Some(u) => u.as_str(),
        None => {
            eprintln!("error: --url is required unless --replay is given");
            return ExitCode::from(1);
        }
    };
    let model = match args.model {
        Some(m) => m,
        None => {
            eprintln!("error: --model is required unless --replay is given");
            return ExitCode::from(1);
        }
    };

    let cfg = match RunConfig::build(
        url,
        model,
        args.conversations,
        args.concurrency,
        args.stream,
        args.max_turns,
        args.max_tokens,
        args.temperature,
        args.timeout,
        args.api_key,
        &args.headers,
        &args.messages,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    #[cfg(not(feature = "tui"))]
    if args.tui {
        eprintln!("error: this build has no TUI support; rebuild with --features tui");
        return ExitCode::from(1);
    }

    let result = if args.tui {
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
            unreachable!("--tui rejected above")
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

    // Optional save.
    if let Some(ref path) = args.output {
        if let Err(e) = persist::save(path, &result) {
            eprintln!("warning: could not save output: {e}");
        }
    }

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
