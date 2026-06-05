# llmprobe — Install & Usage

## Install

`llmprobe` ships as a statically linked Linux binary with no runtime dependencies.

```sh
tar -xzf llmprobe-<version>-x86_64-unknown-linux-musl.tar.gz

# Per-user
install -m 755 llmprobe ~/.local/bin/llmprobe

# Or system-wide
sudo install -m 755 llmprobe /usr/local/bin/llmprobe

llmprobe --version
```

## Quick start

```sh
# Live dashboard — grows conversations until context limit, runs forever
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4

# Fixed run: 10 conversations, 2 concurrent slots, save result
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 10 -c 2 --stream \
         --output run.json

# Replay a saved run interactively (no HTTP requests made)
llmprobe --replay run.json

# Machine-readable JSON report
llmprobe -u "$ENDPOINT" -m my-model -n 20 --stream --json --no-tui
```

## Options

| Flag | Description | Default |
|------|-------------|---------|
| `-u, --url <URL>` | Base or full endpoint. Appends `/v1/chat/completions` if absent. | (required) |
| `-m, --model <NAME>` | Model identifier sent in the request body. | (required) |
| `-n, --conversations <N>` | Total conversations to complete. `0` = run forever. | `0` |
| `-c, --concurrency <C>` | Concurrent conversation slots. | `1` |
| `--stream` | Enable streaming. Measures TTFT and TPOT. Strongly recommended. | off |
| `-s, --system <TEXT>` | Fixed system prompt. Omit to sample randomly from the built-in pool. | random |
| `--max-turns <N>` | Stop a conversation after N turns. `0` = unlimited. | `0` |
| `--max-tokens <N>` | Cap output tokens per turn. Omit to let the server decide. | unset |
| `--seed <N>` | RNG seed for reproducible prompt sequences across runs. | random |
| `--timeout <SECS>` | Per-turn timeout (covers the full stream). | `60` |
| `--api-key <KEY>` | Bearer token. Falls back to `$OPENAI_API_KEY`. | env |
| `-H, --header <K: V>` | Extra HTTP header, repeatable. | — |
| `--no-tui` | Print a plain-text report instead of the live dashboard. | off |
| `--json` | Print a machine-readable JSON report to stdout. | off |
| `--output <FILE>` | Save the completed run to FILE (reopen with `--replay`). | — |
| `--replay <FILE>` | Load a saved run and open the interactive view. No HTTP requests. | — |

## TUI key bindings

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate conversations |
| `g` / `G` | Jump to top / bottom |
| `Enter` | Open conversation detail → turn list |
| `Enter` (in turn list) | Open request / response view |
| `x` (in turn view) | Expand full request payload |
| `Esc` | Close current modal |
| `s` | Cycle sort: Recent → Turns → TTFT → TPS |
| `Space` / `p` | Pause / resume |
| `?` | Help overlay |
| `q` | Quit |

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | All conversations completed without errors. |
| `2` | Some conversations ended with errors. |
| `1` | All conversations errored, config error, or endpoint unreachable. |

## Glossary

| Term | Meaning |
|------|---------|
| **TTFT** | Time to first content token (streaming). Queueing + prefill time. |
| **TPOT** | `(e2e − TTFT) / (tokens − 1)` in ms. Per-step decode latency. Grows as context fills. |
| **TPS** | Tokens/s. *Per-request* = decode rate. *Aggregate* = Σ tokens ÷ wall-clock. |
| **ITL** | Mean inter-token gap in ms (streaming). |
| **ctx-limit** | Conversation hit the model's context window — the expected outcome. |
| **p50 / p95 / p99** | Percentiles across all successful turns. |
