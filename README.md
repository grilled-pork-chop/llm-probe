# llmprobe

A CLI benchmarking tool for OpenAI-compatible `/v1/chat/completions` endpoints
(vLLM, TGI, llama.cpp, Ollama, hosted APIs, …).

Instead of sending isolated requests, llmprobe **grows conversations
turn-by-turn** until the model refuses with a context-overflow error.
This is the most realistic way to stress-test a deployment: it
exercises prefill, decode, KV-cache reuse, and memory pressure at scale.

```
llmprobe -u http://localhost:8000 -m my-model --stream -c 4
```

The live TUI (default) shows TTFT, TPOT, and TPS metrics updating in real time
with per-conversation and per-turn drill-down.

## Key metrics

| Metric | Meaning |
|--------|---------|
| **TTFT** | Time to first content token (streaming only). Queueing + prefill time. |
| **TPOT** | `(e2e − TTFT) / (completion_tokens − 1)` — per-step decode latency in ms. |
| **TPS** | Tokens per second. *Per-request* = decode rate; *aggregate* = deployment throughput. |
| **ITL** | Mean inter-token gap in ms (streaming). Reciprocal of per-request TPS. |
| **e2e** | End-to-end latency: request sent → last byte received. |
| **context depth** | Prompt tokens at the conversation's context limit. |
| **TPOT degradation** | How much slower TPOT gets as context fills (by turn-index bucket). |

## Install

```sh
cargo install --path .
```

Builds with the live TUI by default. To build a smaller binary without it:

```sh
cargo install --path . --no-default-features
```

See [INSTALL.md](INSTALL.md) for pre-built binary installation and static musl builds.

## Quick start

```sh
# Live dashboard — grows conversations until context limit, runs forever
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4

# Fixed run: 10 conversations, 2 concurrent slots, save result
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 10 -c 2 --stream \
         --output run.json

# Replay a saved run interactively (no HTTP requests made)
llmprobe --replay run.json

# Machine-readable report
llmprobe -u "$ENDPOINT" -m my-model -n 20 --stream --json --no-tui
```

## How it works

Each concurrent **slot** runs an independent conversation:
1. A random system prompt and seed message are drawn from the built-in
   ShareGPT-derived prompt pool.
2. The conversation grows one user turn at a time, alternating with the model's
   reply.
3. When the server returns a context-overflow error (HTTP 400/413 with a
   recognised message), the conversation is recorded as `ctx-limit` and a new
   one starts.
4. With `--max-turns` the conversation is capped early (useful for warmup or
   fixed-depth benchmarks).

Use `--seed` to fix the RNG and get identical prompt sequences across runs for
fair A/B comparisons.

## Options

See [USAGE.md](USAGE.md) for the full option reference and examples.

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | All conversations completed without errors. |
| `2` | Some conversations ended with errors. |
| `1` | All conversations errored, config error, or endpoint unreachable. |

## Building & portability

- TLS is **rustls** (no system OpenSSL) — cross-compiles cleanly everywhere.
- Release profile is size-optimised (`lto`, `codegen-units = 1`, `strip`).

### Fully static Linux binary (musl)

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --features tui --target x86_64-unknown-linux-musl
```

If your environment lacks `cmake` (needed by `aws-lc-rs` for musl), use
[`cross`](https://github.com/cross-rs/cross):

```sh
cargo install cross
cross build --release --features tui --target x86_64-unknown-linux-musl
```

## License

MIT
