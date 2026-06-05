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

The live TUI (default) shows TTFT, TPOT, and TPS metrics updating in real time,
with a running list of conversations. A full text or JSON report is printed when
the run ends.

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

Download a prebuilt static Linux x86_64 binary from the
[Releases](../../releases) page, or build from source:

```sh
cargo install --path .                       # with the live TUI (default)
cargo install --path . --no-default-features # smaller binary, no TUI
```

See [INSTALL.md](INSTALL.md) for the packaged-binary instructions and full option
reference.

## Quick start

```sh
# Live dashboard — grows conversations until context limit, runs forever
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4

# Fixed run: 10 conversations, 2 concurrent slots
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 10 -c 2 --stream

# Machine-readable report
llmprobe -u "$ENDPOINT" -m my-model -n 20 --stream --json
```

## How it works

Each concurrent **slot** runs an independent conversation:
1. A system prompt and seed message are drawn from a small built-in prompt set.
2. The conversation grows one user turn at a time — the model's reply plus a
   follow-up nudge — so each turn sends a longer, unique context.
3. When the server returns a context-overflow error (HTTP 400/413 with a
   recognised message), the conversation is recorded as `ctx-limit` and a new
   one starts.
4. With `--max-turns` the conversation is capped early (useful for warmup or
   fixed-depth benchmarks).

Use `--seed` to fix the RNG and get identical prompt sequences across runs for
fair A/B comparisons.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks on every push and pull request. Tagging a release
(`git tag v0.1.0 && git push --tags`) builds and publishes the static Linux
binary — the tag must match the `Cargo.toml` version.

## License

MIT — see [LICENSE](LICENSE).
