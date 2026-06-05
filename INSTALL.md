# Installing llmprobe

## From source (recommended)

Requires Rust 1.85+ (edition 2024).

```sh
git clone https://github.com/grilled-pork-chop/llm-probe
cd llm-probe
cargo install --path .
```

This builds with the live TUI enabled (the default feature). To build without
the TUI dashboard (smaller binary, fewer dependencies):

```sh
cargo install --path . --no-default-features
```

Verify the installation:

```sh
llmprobe --version
```

## From a pre-built release tarball

`llmprobe` ships as a statically linked Linux binary (`x86_64-unknown-linux-musl`)
with no runtime dependencies.

```sh
tar -xzf llmprobe-<version>-x86_64-unknown-linux-musl.tar.gz
cd llmprobe-<version>-x86_64-unknown-linux-musl

./llmprobe --version
```

Install to `PATH`:

```sh
# Per-user
install -m 755 llmprobe ~/.local/bin/llmprobe

# System-wide
sudo install -m 755 llmprobe /usr/local/bin/llmprobe
```

Confirm it is statically linked (optional):

```sh
ldd ./llmprobe   # should print "not a dynamic executable" or "statically linked"
file ./llmprobe  # should include "static-pie linked"
```

## Building a static musl binary yourself

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --features tui --target x86_64-unknown-linux-musl
# output: target/x86_64-unknown-linux-musl/release/llmprobe
```

`rustls` uses `aws-lc-rs` which requires a C toolchain and `cmake` for musl
targets. If those are unavailable, use [`cross`](https://github.com/cross-rs/cross):

```sh
cargo install cross
cross build --release --features tui --target x86_64-unknown-linux-musl
```

## Uninstall

```sh
# If installed with cargo install
cargo uninstall llmprobe

# If installed manually
rm -f ~/.local/bin/llmprobe
# or wherever you placed it
```

---

For usage examples and command-line options, see [USAGE.md](USAGE.md).
