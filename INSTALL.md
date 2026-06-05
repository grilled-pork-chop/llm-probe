# llmprobe — Install

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

# Machine-readable JSON report, no TUI
llmprobe -u "$ENDPOINT" -m my-model -n 20 --stream --json --no-tui

# Hosted endpoint with API key
OPENAI_API_KEY=sk-... \
llmprobe -u https://api.example.com/v1 -m gpt-4o-mini -n 10 --stream
```

Run `llmprobe --help` for the full option reference.

## Uninstall

```sh
rm -f ~/.local/bin/llmprobe
```
