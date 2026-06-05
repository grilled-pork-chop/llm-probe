# llmprobe — build & packaging
#
# Common targets:
#   make build     optimized native binary (dynamically linked, fast to build)
#   make static    fully static, portable binary (musl) — for distribution
#   make package   static binary + docs bundled into dist/<name>.tar.gz
#   make install   install the static binary to $(PREFIX)/bin
#   make check     fmt + clippy + tests
#   make help      list all targets

CARGO   ?= cargo
TARGET  ?= x86_64-unknown-linux-musl
PREFIX  ?= $(HOME)/.local

NAME    := llmprobe
VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
BINDIR  := $(PREFIX)/bin

BIN     := target/$(TARGET)/release/$(NAME)
DIST    := dist
PKGNAME := $(NAME)-$(VERSION)-$(TARGET)
PKGDIR  := $(DIST)/$(PKGNAME)
TARBALL := $(DIST)/$(PKGNAME).tar.gz

.PHONY: all build static package install uninstall test fmt clippy check clean help

all: build

## build: optimized native release binary (dynamically linked)
build:
	$(CARGO) build --release

## static: fully static, portable binary via musl (no runtime deps)
static:
	rustup target add $(TARGET) >/dev/null 2>&1 || true
	$(CARGO) build --release --target $(TARGET)

## package: build the static binary and bundle it with the docs into a .tar.gz
package: static INSTALL.md README.md LICENSE
	@rm -rf $(PKGDIR)
	@mkdir -p $(PKGDIR)
	cp $(BIN) $(PKGDIR)/$(NAME)
	cp INSTALL.md $(PKGDIR)/
	cp README.md $(PKGDIR)/
	cp LICENSE $(PKGDIR)/
	tar -czf $(TARBALL) -C $(DIST) $(PKGNAME)
	@rm -rf $(PKGDIR)
	@echo "==> packaged $(TARBALL)"
	@ls -lh $(TARBALL)

## install: install the static binary to $(BINDIR)
install: static
	install -d $(BINDIR)
	install -m 755 $(BIN) $(BINDIR)/$(NAME)
	@echo "==> installed $(NAME) $(VERSION) to $(BINDIR)/$(NAME)"

## uninstall: remove the installed binary
uninstall:
	rm -f $(BINDIR)/$(NAME)
	@echo "==> removed $(BINDIR)/$(NAME)"

## test: run the full test suite (all features)
test:
	$(CARGO) test --all-features

## fmt: check formatting
fmt:
	$(CARGO) fmt --check

## clippy: lint with warnings treated as errors
clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

## check: fmt + clippy + tests
check: fmt clippy test

## clean: remove build artifacts and the dist directory
clean:
	$(CARGO) clean
	rm -rf $(DIST)

## help: list available targets
help:
	@grep -E '^## ' Makefile | sed -e 's/^## //'
