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

## Uninstall

```sh
rm -f ~/.local/bin/llmprobe
```
