# llmprobe

A minimal, portable CLI that smoke-tests any OpenAI-compatible
`/v1/chat/completions` endpoint. It fires a configurable batch of requests
(optional concurrency + streaming), measures latency / TTFT / TPS / error rate,
and prints a clean terminal report. An optional live TUI shows the same metrics
updating in real time.

Token counts come from the API's `usage` field (no client-side tokenizer).
Durations use a monotonic clock. One shared, connection-pooled HTTP client backs
the whole batch so concurrency numbers aren't dominated by TLS handshakes.

## Install

```sh
cargo install --path .
```

This builds with the TUI by default. For a smaller, dependency-light binary
without the dashboard:

```sh
cargo install --path . --no-default-features
```

`--tui` still parses on a no-TUI build; it just exits with an error telling you
to rebuild with `--features tui`.

## Usage

```text
llmprobe --url <BASE_URL> --model <NAME> [options]

  -u, --url <URL>          Base or full endpoint. If it doesn't end in
                           /chat/completions, that path is appended.
  -m, --model <NAME>       Model name (required)
  -n, --requests <N>       Number of requests; 0 = run forever  [default: 0]
  -c, --concurrency <C>    Max in-flight requests         [default: 1]
      --stream             Enable streaming + TTFT measurement
  -p, --prompt <TEXT>      Prompt          [default: a short fixed prompt]
      --max-tokens <N>     Cap output tokens              [default: 128]
      --temperature <F>    Sampling temperature             [optional]
      --timeout <SECS>     Per-request timeout            [default: 30]
      --warmup <K>         Discard the first K requests    [default: 0]
      --api-key <KEY>      Bearer token (else $OPENAI_API_KEY)
  -H, --header <K:V>       Extra header (repeatable)
      --tui                Live dashboard (requires the `tui` feature)
      --json               Machine-readable report
      --no-color           Disable ANSI color
```

By default (`-n 0`) llmprobe runs **indefinitely** until interrupted — `q` in
the TUI or `Ctrl-C` in plain mode — then prints the final report. Pass `-n <N>`
for a fixed batch of N requests.

### Examples

```sh
# Continuous live monitor (default — runs until you quit)
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4 --tui

# Fixed batch: 20 non-streaming requests, 4 in parallel
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 20 -c 4

# Fixed batch, machine-readable report
llmprobe -u "$ENDPOINT" -m my-model -n 50 --json
```

### Notes

- The request sends **`max_tokens`** (widest support across vLLM / TGI /
  llama.cpp / Ollama). OpenAI's newest models prefer `max_completion_tokens`;
  that is not handled in v0.1.
- `--timeout` is a **total** per-request deadline covering the whole stream, so
  it must exceed expected full generation time for long outputs.
- When streaming, `stream_options.include_usage` is requested so the final SSE
  chunk carries `usage`; if a server ignores it, TPS becomes `—` rather than an
  error.

## Exit codes

| Code | Meaning |
|------|---------|
| `0`  | all requests succeeded |
| `2`  | partial failures (some requests failed) |
| `1`  | total failure, config error, or endpoint unreachable |

## Glossary

| Term | Plain-English meaning |
|---|---|
| **e2e latency** | Full round trip — send to last byte back. |
| **TTFT** | Time to first token: the wait before the first generated token (streaming). |
| **TPS** | Tokens/s. *per req* = one request's decode rate; *aggregate* = total system throughput. |
| **ITL / inter-token** | Mean gap between output tokens (ms). Lower = smoother. Reciprocal of per-req TPS. |
| **max gap** | Longest pause between two tokens in a request — catches stalls an average hides. |
| **p50 / p95 / p99** | Half / 95% / 99% of requests were faster than this. p95/p99 are the tail. |
| **min / avg / max** | Fastest / mean / slowest sample. |
| **jitter (±)** | Std-dev of latency — how consistent the endpoint is. |
| **req/s** | Completed requests per second across the run. |
| **c / concurrency** | How many requests are allowed in flight at once. |
| **in-flight / live** | Requests currently open, awaiting a response. |
| **speedup** | Aggregate TPS ÷ per-request TPS (≈1 = no parallel scaling). |
| **output len** | Completion tokens per request — spots empty, refused, or truncated replies. |
| **warmup** | Initial throwaway requests excluded from stats, to remove cold-start skew. |

Symbols: `●` in flight · `✓` succeeded (2xx) · `✗ <code>` failed (cause shown).

## Measurement semantics

- **e2e latency** = response complete − just before send (per request).
- **TTFT** (streaming) = first non-empty `delta.content` − just before send.
- **TPS, streaming** = decode throughput = `completion_tokens / (last_token −
  first_token)` — isolates generation speed from queueing/TTFT.
- **TPS, non-streaming** = `completion_tokens / e2e` (no per-token timing; the
  report labels the two differently rather than conflating them).
- **Aggregate TPS** = total completion tokens ÷ wall-clock — the deployment's
  throughput under the chosen concurrency.
- **Percentiles** use nearest-rank; with small N they are coarse and the report
  says so.

## TUI keys

`↑/↓` `j/k` `g`/`G` select a request · `enter` inspect the selected request
(full request + response) · `space` / `p` pause sending and freeze the view (read
steady numbers; press again to resume) · `?` help + glossary · `q` / `Esc` /
`Ctrl-C` quit (restores the terminal, then prints the report). The dashboard
reads the exact same measurement stream as plain mode, so the numbers can't
diverge between modes.

## Building & portability

- TLS is **rustls** (no system OpenSSL), so the tool cross-compiles cleanly and
  has no OpenSSL runtime dependency.
- Release profile is size-optimized (`lto`, `codegen-units = 1`, `strip`).

### Fully static Linux binary (musl)

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

`rustls` uses the `aws-lc-rs` provider, which needs a C toolchain (and `cmake`)
to build for musl. If your environment lacks one, the simplest path is the
[`cross`](https://github.com/cross-rs/cross) tool:

```sh
cargo install cross
cross build --release --target x86_64-unknown-linux-musl
```

## License

MIT (or your choice — fill in before publishing).
