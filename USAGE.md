# Using llmprobe

`llmprobe` grows multi-turn conversations against an OpenAI-compatible
`/v1/chat/completions` endpoint, measuring TTFT, TPOT, TPS, and context-window
behaviour. It is designed to stress-test LLM deployments under realistic,
cache-busting traffic that naturally fills the context window.

```sh
llmprobe --url <BASE_URL> --model <NAME> [options]
```

## Options

| Flag | Description | Default |
|------|-------------|---------|
| `-u, --url <URL>` | Base or full endpoint. Appends `/v1/chat/completions` if absent. | (required) |
| `-m, --model <NAME>` | Model identifier sent in the request body. | (required) |
| `-n, --conversations <N>` | Total conversations to complete across all slots. `0` = run forever. | `0` |
| `-c, --concurrency <C>` | Number of concurrent conversation slots (virtual users). | `1` |
| `--stream` | Enable streaming. Measures TTFT and TPOT. Strongly recommended. | off |
| `-s, --system <TEXT>` | Fixed system prompt for every conversation. Omit to sample randomly from the built-in pool. | random |
| `--max-turns <N>` | Stop a conversation after N turns regardless of context limit. `0` = unlimited. | `0` |
| `--max-tokens <N>` | Cap output tokens per turn. Omit to let the server decide. | unset |
| `--seed <N>` | RNG seed for reproducible prompt selection. Same seed → same prompt sequences. | random |
| `--timeout <SECS>` | Per-turn request timeout (total, covers the full stream). | `60` |
| `--api-key <KEY>` | Bearer token. Falls back to `$OPENAI_API_KEY`. | env |
| `-H, --header <K: V>` | Extra HTTP header, repeatable (e.g. `-H "X-Org: research"`). | — |
| `--no-tui` | Disable the live TUI dashboard. Prints a plain-text report instead. | off |
| `--json` | Print a machine-readable JSON report to stdout instead of plain text. | off |
| `--output <FILE>` | Save the completed run to FILE as JSON (reopen with `--replay`). | — |
| `--replay <FILE>` | Load a saved run from FILE and open the interactive view. No HTTP requests made. | — |

The URL is normalised: `http://host:8000`, `http://host:8000/v1`, and
`http://host:8000/v1/chat/completions` all resolve to the same endpoint.

## How conversations work

Each concurrent slot runs an independent loop:

1. **System prompt** — drawn randomly from a 25-persona pool (verbose, expert
   styles chosen to produce long replies) unless `--system` overrides it.
2. **Seed turn** — first user message drawn randomly from 34 topic categories
   (Python, databases, distributed systems, ML, …), each with 15 follow-up
   variants.
3. **Growth** — each successful turn appends the assistant reply and a new
   follow-up from the same category. The conversation grows by ~2× tokens per
   turn.
4. **Terminal** — the slot records the outcome and starts the next conversation:
   - `ctx-limit` — server refused with a context-overflow error (the goal).
   - `max-turns` — `--max-turns` cap reached.
   - `error(…)` — unexpected HTTP/timeout/decode error.

Use `--seed` to reproduce an exact sequence across runs for fair comparison.

## Examples

```sh
# Live dashboard — run forever, 4 slots, streaming
llmprobe -u http://localhost:8000 -m llama-3.1-8b --stream -c 4

# Fixed run: 20 conversations, save result for later review
llmprobe -u http://localhost:8000 -m llama-3.1-8b -n 20 -c 4 --stream \
         --output bench.json

# Replay a saved run interactively (no HTTP requests made)
llmprobe --replay bench.json

# Hosted endpoint with API key and extra header
OPENAI_API_KEY=sk-... \
llmprobe -u https://api.example.com/v1 -m gpt-4o-mini -n 10 --stream \
         -H "X-Org: myorg"

# Machine-readable output, no TUI
llmprobe -u "$ENDPOINT" -m my-model -n 20 --stream --json --no-tui

# Reproducible run (same prompts every time)
llmprobe -u http://localhost:8000 -m my-model -n 10 --stream --seed 42

# Cap conversations at 5 turns each (useful for quick smoke tests)
llmprobe -u http://localhost:8000 -m my-model -c 4 --stream --max-turns 5
```

## Reading the plain-text report (`--no-tui`)

```
llmprobe — http://localhost:8000/v1/chat/completions  (model: llama-3.1-8b)
mode: streaming · concurrency: 4

Conversations  10 total  10 ctx-limit  0 errors
  turns-to-limit   min 8  p50 12  p95 18  p99 19  max 20
  context depth    min 12,048  p50 15,300  p95 16,100  max 16,384  tok

TTFT    p50 142 ms  p95 218 ms  p99 261 ms  avg 150 ms

TPOT    p50 11.2 ms  p95 17.4 ms  p99 22.1 ms  avg 12.0 ms
  ITL   avg 11.2 ms  p95 17.3 ms

TPS (per-req)  p50 89.3  p95 91.2  p99 92.0  avg 88.8 tok/s
  aggregate    347.2 tok/s

Throughput  success 100.0%  ok/total 120/120  compl-tokens 61,440
  output len  min 384  avg 512  max 640 tok
  e2e         p50 5.8 s  p95 8.2 s  avg 5.9 s

TPOT degradation by turn index (context growth effect)
  turns 1-4    11.2 ms
  turns 5-8    13.1 ms   +17%
  turns 9-12   16.4 ms   +46%
  turns 13-16  21.0 ms   +87%
```

The **degradation table** shows how per-token decode latency grows as the
context window fills — the primary indicator of KV-cache memory pressure.

## Live TUI

The TUI is enabled by default. Disable with `--no-tui`.

### Layout

```
┌─ header ───────────────────────────────────────────────────────────────────┐
│ endpoint · model · concurrency · elapsed                          [LIVE]   │
├─ tiles ─────────────────────────────────────────────────────────────────────┤
│  TTFT p50/p95/p99 │ TPOT p50/p95/p99 │ TPS p50/agg │ Status │ Throughput  │
├─ sparklines ────────────────────────────────────────────────────────────────┤
│  TPS history ▁▂▄▆▇█                  │  TTFT history ▁▂▃▄▅                │
├─ conversations ─────────────────────────────────────────────────────────────┤
│  # │ slot │ turns │ state │ TTFT avg │ TPS avg │ prompt-tok │ terminal     │
│  …                                                                          │
└─ footer ───────────────────────────────────────────────────────────────────┘
```

### Key bindings

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Select conversation / scroll |
| `g` / `G` | Jump to top / bottom |
| `Enter` | Open conversation detail (turn list) |
| `Enter` (in detail) | Open turn request/response view |
| `x` (in turn view) | Expand full request payload (all messages) |
| `Esc` | Close current modal (step back one level) |
| `s` | Cycle sort: Recent → Turns → TTFT → TPS |
| `Space` / `p` | Pause / resume (freeze metrics, stop sending) |
| `?` | Toggle help overlay |
| `q` | Quit |

### Modals

**Conversation detail** (`Enter` on main table): shows every turn with its
prompt/completion token counts, e2e, TTFT, TPOT, and TPS. The final turn
shows the terminal reason.

**Turn view** (`Enter` on a turn row): shows the exact request payload and
model reply (or error body). Press `x` to expand from "last user message only"
to the full conversation history.

### Saving and replaying

Save a completed run to review later without re-running:

```sh
llmprobe -u http://localhost:8000 -m my-model -n 20 --stream \
         --output run.json

llmprobe --replay run.json
```

The replay view is identical to the completed live view — full conversation
table, sort, drill-down, and key navigation all work normally.

## Glossary

| Term | Meaning |
|------|---------|
| **TTFT** | Time to first content token (streaming only). Includes queueing + prefill. |
| **TPOT** | `(e2e − TTFT) / (tokens − 1)`. Per-step decode latency in ms. Grows as context fills. |
| **TPS** | Tokens/s. *Per-request* = decode rate. *Aggregate* = Σ tokens ÷ wall-clock. |
| **ITL** | Inter-token latency — mean gap between successive output tokens (streaming). |
| **e2e latency** | Full round trip: request sent → last byte received. |
| **context depth** | Prompt tokens at the point the context limit was hit. |
| **ctx-limit** | Conversation ended because the server returned a context-overflow error — the expected outcome. |
| **TPOT degradation** | Percentage increase in TPOT between early and late turns, as context fills the KV cache. |
| **p50 / p95 / p99** | Percentiles across all successful turns. p95/p99 show the tail. |
| **concurrency / slots** | Number of conversations growing simultaneously. |

Token counts come from the API's `usage` field (no client-side tokeniser).
Durations use a monotonic clock. Pause time is excluded from elapsed and
aggregate-TPS calculations.
