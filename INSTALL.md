# Installing llmprobe

`llmprobe` is distributed as a single self-contained binary.

The release build is a statically linked Linux executable (`x86_64-unknown-linux-musl`) with no runtime dependencies.

## Install from a release tarball

```sh
tar -xzf llmprobe-<version>-x86_64-unknown-linux-musl.tar.gz
cd llmprobe-<version>-x86_64-unknown-linux-musl

./llmprobe --version
```

To make `llmprobe` available system-wide:

```sh
# Per-user installation
install -m 755 llmprobe ~/.local/bin/llmprobe

# Or system-wide
sudo install -m 755 llmprobe /usr/local/bin/llmprobe
```

## Verify the installation

Check that the binary runs correctly:

```sh
llmprobe --version
```

Confirm that it is statically linked:

```sh
ldd ./llmprobe
# Expected output:
# not a dynamic executable
# or
# statically linked

file ./llmprobe
# Expected output includes:
# ELF 64-bit ... static-pie linked
```

## Uninstall

```sh
rm -f ~/.local/bin/llmprobe
```

Or remove it from the location where it was installed.

For usage examples and command-line options, see `USAGE.md`.
