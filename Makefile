# Optional developer convenience targets.  ASB runtime and release workflows do
# not depend on make being installed.

SHELL := /bin/sh
.DEFAULT_GOAL := help

CARGO ?= cargo
RUSTUP ?= rustup
GIT ?= git
MAKE ?= make
RUSTUP_TOOLCHAIN ?= 1.93.0
TARGET ?= x86_64-unknown-linux-gnu
PREFIX ?= $(CURDIR)/.make/install
CARGO_TARGET_DIR ?= $(CURDIR)/target

.PHONY: help check-deps build install clean update test

help:
	@printf '%s\n' \
		'ASB optional developer targets (Make is never needed at runtime):' \
		'  make check-deps  Verify pinned Rust, target, and host tools (no installs)' \
		'  make build       Build the locked workspace' \
		'  make test        Run format, Clippy, tests, and rustdoc gates' \
		'  make install     Install asb into PREFIX (default: .make/install)' \
		'  make clean       Remove only Cargo target and marked Make staging data' \
		'  make update      Fast-forward Git and validate the locked dependency set' \
		'' \
		'Overrides: CARGO, RUSTUP, GIT, RUSTUP_TOOLCHAIN, TARGET, PREFIX, CARGO_TARGET_DIR'

check-deps:
	@set -eu; \
	for tool in "$(CARGO)" "$(RUSTUP)" "$(GIT)" "$(MAKE)"; do \
		command -v "$$tool" >/dev/null 2>&1 || { \
			printf '%s\n' "ERROR: missing '$$tool'; install it using your OS/toolchain policy (Make does not install dependencies)." >&2; exit 1; \
		}; \
	done; \
	"$(RUSTUP)" toolchain list | awk -v wanted='$(RUSTUP_TOOLCHAIN)' '$$1 == wanted { found=1 } END { if (!found) exit 1 }' || { \
		printf '%s\n' "ERROR: Rust toolchain $(RUSTUP_TOOLCHAIN) is unavailable; install it with rustup outside this Makefile." >&2; exit 1; \
	}; \
	"$(RUSTUP)" target list --toolchain "$(RUSTUP_TOOLCHAIN)" --installed | awk -v wanted='$(TARGET)' '$$1 == wanted { found=1 } END { if (!found) exit 1 }' || { \
		printf '%s\n' "ERROR: Rust target $(TARGET) is unavailable for $(RUSTUP_TOOLCHAIN); add it with rustup outside this Makefile." >&2; exit 1; \
	}; \
	printf '%s\n' "Dependencies ready: Rust $(RUSTUP_TOOLCHAIN), target $(TARGET), Cargo, rustup, Git, and Make."

build: check-deps
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" build --locked --workspace --target "$(TARGET)"

install: check-deps
	@set -eu; \
	root=$$(pwd -P); case "$(PREFIX)" in /*) prefix=$(PREFIX);; *) prefix=$$root/$(PREFIX);; esac; \
	[ -n "$$prefix" ] || { printf '%s\n' "ERROR: PREFIX must name a creatable directory." >&2; exit 1; }; \
	[ "$$prefix" != / ] && [ "$$prefix" != "$$root" ] || { printf '%s\n' 'ERROR: refusing unsafe PREFIX (filesystem or repository root).' >&2; exit 1; }; \
	case "$$prefix" in "$$root"/.make/*) ;; *) printf '%s\n' 'ERROR: PREFIX must be below the repository .make staging directory.' >&2; exit 1 ;; esac; \
	mkdir -p -- "$$prefix"; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" install --locked --path crates/asb-cli --root "$$prefix"; \
	touch -- "$$prefix/.asb-make-staging"

clean:
	@set -eu; \
	root=$$(pwd -P); case "$(CARGO_TARGET_DIR)" in /*) target=$(CARGO_TARGET_DIR);; *) target=$$root/$(CARGO_TARGET_DIR);; esac; \
	[ -n "$$target" ] && [ "$$target" != / ] && [ "$$target" != "$$root" ] || { printf '%s\n' 'ERROR: refusing unsafe CARGO_TARGET_DIR.' >&2; exit 1; }; \
	case "$$target" in "$$root"/*|/tmp/*) ;; *) printf '%s\n' 'ERROR: CARGO_TARGET_DIR must be repository-local or under /tmp.' >&2; exit 1 ;; esac; \
	if [ -d "$(PREFIX)" ] && [ -f "$(PREFIX)/.asb-make-staging" ]; then rm -rf -- "$(PREFIX)"; fi; \
	if [ "$$target" != "$$root/target" ] || [ -d "$$target" ]; then rm -rf -- "$$target"; fi

update: check-deps
	@set -eu; \
	"$(GIT)" diff --quiet && "$(GIT)" diff --cached --quiet || { printf '%s\n' 'ERROR: update requires a clean source tree (commit or stash changes first).' >&2; exit 1; }; \
	"$(GIT)" pull --ff-only; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" update --locked

test: check-deps
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" fmt --all -- --check
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" clippy --locked --workspace --all-targets --target "$(TARGET)" -- -D warnings
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" test --locked --workspace --target "$(TARGET)"
	@RUSTDOCFLAGS='-D warnings' CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" doc --locked --workspace --no-deps --target "$(TARGET)"
