# Using llmprobe

`llmprobe` fires a batch of requests at an OpenAI-compatible
`/v1/chat/completions` endpoint and reports latency, time-to-first-token,
tokens/second, and error rate. It works as a one-shot smoke test, a CI gate, or
a live dashboard.

```sh
llmprobe --url <BASE_URL> --model <NAME> [options]
```

## Options

| Flag                    | Description                                                                             | Default              |
| ----------------------- | --------------------------------------------------------------------------------------- | -------------------- |
| `-u, --url <URL>`       | Base or full endpoint. If it doesn't end in `/chat/completions`, that path is appended. | (required)           |
| `-m, --model <NAME>`    | Model name.                                                                             | (required)           |
| `-n, --requests <N>`    | Number of requests. `0` runs **indefinitely** until interrupted.                        | `0` (infinite)       |
| `-c, --concurrency <C>` | Max requests in flight at once.                                                         | `1`                  |
| `--stream`              | Enable streaming + TTFT / decode metrics.                                               | off                  |
| `-p, --prompt <TEXT>`   | Prompt text.                                                                            | a short fixed prompt |
| `--max-tokens <N>`      | Cap output tokens.                                                                      | `128`                |
| `--temperature <F>`     | Sampling temperature (omitted when unset).                                              | unset                |
| `--timeout <SECS>`      | Per-request timeout (total, covers the whole stream).                                   | `30`                 |
| `--warmup <K>`          | Discard the first K requests (excludes cold-start skew).                                | `0`                  |
| `--api-key <KEY>`       | Bearer token. Falls back to `$OPENAI_API_KEY`.                                          | none                 |
| `-H, --header <K:V>`    | Extra header, repeatable.                                                               | —                    |
| `--tui`                 | Live dashboard (requires the `tui` feature).                                            | off                  |
| `--json`                | Machine-readable report to stdout.                                                      | off                  |
| `--no-color`            | Disable ANSI color.                                                                     | auto                 |

The URL is normalized: `http://host:8000`, `http://host:8000/v1`, and
`http://host:8000/v1/chat/completions` all resolve to the same endpoint.

## Examples

```sh
# Continuous live monitor (default): runs until you quit — best with --tui
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4 --tui

# Fixed batch: 20 non-streaming requests, 4 in parallel, then stop
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 20 -c 4

# Hosted endpoint with auth and a custom header, fixed batch
OPENAI_API_KEY=sk-... \
llmprobe -u https://api.example.com/v1 -m gpt-4o-mini -n 30 \
         -H "X-Org: research"

# Machine-readable report for a fixed batch
llmprobe -u "$ENDPOINT" -m my-model -n 50 --json

# Warm up first, then measure a fixed batch
llmprobe -u http://localhost:8000 -m my-model -n 50 --warmup 5
```

Without `-n` (or with `-n 0`) llmprobe runs **indefinitely** until you stop it:
press `q` in the TUI, or `Ctrl-C` in plain mode — either way it prints the final
report on exit.

## Reading the report

```text
llmprobe — http://localhost:8000/v1/chat/completions  (model: llama-3.1-8b)
mode: streaming · requests: 20 · concurrency: 4 · max_tokens: 128

Requests    20 total   20 ok   0 failed   (100.0% success)
Wall clock  4.21 s     throughput 4.75 req/s
End-to-end latency
  min 0.84 s   p50 1.62 s   avg 1.71 s   p95 2.93 s   p99 3.08 s   max 3.10 s  (±0.41)
Time to first token
  min 0.12 s   avg 0.21 s   p95 0.38 s
Throughput
  tokens/s   avg/req 78.4    aggregate 312.6    inter-token 12.4 ms (max gap 41 ms)
  completion 2,560 tok total   ·   output len  min 96  avg 128  max 128
  concurrency speedup 3.7×
```

- An `Errors` section lists failures by class when any occur (e.g. `3× HTTP 429`).
- With fewer than 20 requests a note flags that percentiles are approximate.
- Color is on when stdout is a TTY; disabled by `--no-color`, `NO_COLOR`, or `--json`.

### Glossary

| Term                  | Meaning                                                                          |
| --------------------- | -------------------------------------------------------------------------------- |
| **e2e latency**       | Full round trip — send to last byte back.                                        |
| **TTFT**              | Time to first token: the wait before the first generated token (streaming only). |
| **TPS** (tokens/s)    | *per req* = one request's decode rate; *aggregate* = total system throughput.    |
| **inter-token / ITL** | Mean gap between output tokens (ms). Lower = smoother.                           |
| **max gap**           | Longest pause between two tokens — catches stalls an average hides.              |
| **p50 / p95 / p99**   | Half / 95% / 99% of requests were faster than this. p95/p99 are the tail.        |
| **jitter (±)**        | Std-dev of latency — how consistent the endpoint is.                             |
| **req/s**             | Completed requests per second across the run.                                    |
| **speedup**           | Aggregate TPS ÷ per-request TPS (≈1 = no parallel scaling, ≈C = good scaling).   |
| **output len**        | Completion tokens per request — spots empty, refused, or truncated replies.      |
| **warmup**            | Initial throwaway requests excluded from stats.                                  |

Token counts come from the API's `usage` field. Durations use a monotonic clock.


## Live TUI

`--tui` opens an `htop`-style dashboard: progress gauge, throughput / latency /
first-token tiles, moving TPS and TTFT sparklines, an e2e distribution chart,
and a live request table.

| Key                    | Action                                                                                                                                               |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `↑/↓` `j/k` `g`/`G`    | select a request (top / bottom)                                                                                                                      |
| `enter`                | inspect the selected request — opens a panel with the exact JSON sent and the model's reply (or the error body); `↑/↓` scrolls it, `esc` closes      |
| `space` / `p`          | **pause / resume** — stops sending new requests (the endpoint rests) and freezes the live view so you can read steady numbers; press again to resume |
| `?`                    | toggle help + glossary overlay                                                                                                                       |
| `q` / `Esc` / `Ctrl-C` | quit — restores the terminal, then prints the report                                                                                                 |
