# Optional developer convenience targets.  ASB runtime and release workflows do
# not depend on make being installed.

SHELL := /bin/sh
.DEFAULT_GOAL := lifecycle

CARGO ?= cargo
RUSTUP ?= rustup
GIT ?= git
MAKE ?= make
RUSTUP_TOOLCHAIN ?= 1.93.0
TARGET ?= x86_64-unknown-linux-gnu
PREFIX ?= $(HOME)/.local
CARGO_TARGET_DIR ?= $(CURDIR)/target

.PHONY: help lifecycle check-deps build install clean update refresh-lock test

help:
	@printf '%s\n' \
		'ASB optional developer targets (Make is never needed at runtime):' \
		'  make            Run update, build, test, then install (development channel)' \
		'  make check-deps  Verify pinned Rust, target, and host tools (no installs)' \
		'  make build       Build the locked workspace' \
		'  make test        Run format, Clippy, tests, and rustdoc gates' \
		'  make install     Install asb into PREFIX (default: $$HOME/.local/bin/asb)' \
		'  make clean       Remove only Cargo target and marked Make staging data' \
		'  make update      Fast-forward Git and validate Cargo.lock without changing it' \
		'  make refresh-lock  Deliberately refresh Cargo.lock, then validate it (review changes)' \
		'' \
		'Overrides: CARGO, RUSTUP, GIT, RUSTUP_TOOLCHAIN, TARGET, PREFIX, CARGO_TARGET_DIR'

lifecycle: update build test install

check-deps:
	@set -eu; \
	for tool in "$(CARGO)" "$(RUSTUP)" "$(GIT)" "$(MAKE)"; do \
		command -v "$$tool" >/dev/null 2>&1 || { \
			printf '%s\n' "ERROR: missing '$$tool'; install it using your OS/toolchain policy (Make does not install dependencies)." >&2; exit 1; \
		}; \
	done; \
	"$(RUSTUP)" toolchain list | awk -v wanted='$(RUSTUP_TOOLCHAIN)' '$$1 == wanted || index($$1, wanted "-") == 1 { found=1 } END { if (!found) exit 1 }' || { \
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
	root=$$(pwd -P); prefix="$(PREFIX)"; \
	case "$$prefix" in /*) ;; *) printf '%s\n' 'ERROR: PREFIX must be an absolute path (for example $$HOME/.local).' >&2; exit 1 ;; esac; \
	case "$$prefix" in *'/../'*|../*|*/..|..) printf '%s\n' 'ERROR: PREFIX must not contain parent-directory traversal.' >&2; exit 1 ;; esac; \
	[ "$$prefix" != / ] && [ "$$prefix" != /.local ] && [ "$$prefix" != "$$root" ] || { printf '%s\n' 'ERROR: refusing unsafe PREFIX (filesystem or repository root).' >&2; exit 1; }; \
	reject_symlink_chain() { path=$$1; rest=$${path#/}; current=/; while [ -n "$$rest" ]; do component=$${rest%%/*}; [ "$$rest" = "$$component" ] && rest= || rest=$${rest#*/}; current="$$current$$component"; [ ! -L "$$current" ] || { printf '%s\n' "ERROR: refusing symlink in PREFIX path: $$current" >&2; exit 1; }; [ ! -e "$$current" ] || [ -d "$$current" ] || { printf '%s\n' "ERROR: PREFIX path component is not a directory: $$current" >&2; exit 1; }; current="$$current/"; done; }; \
	reject_symlink_chain "$$prefix"; \
	mkdir -p -- "$$prefix"; \
	reject_symlink_chain "$$prefix/bin"; \
	[ ! -L "$$prefix/bin/asb" ] || { printf '%s\n' 'ERROR: refusing symlink at PREFIX/bin/asb.' >&2; exit 1; }; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" install --locked --path crates/asb-cli --root "$$prefix"; \
	touch -- "$$prefix/.asb-make-staging"; \
	printf '%s\n' "Installed ASB at $$prefix/bin/asb"; \
	case ":$${PATH:-}:" in *:"$$prefix/bin":*) ;; *) printf '%s\n' "Add $$prefix/bin to PATH to run asb." ;; esac

clean:
	@set -eu; \
	root=$$(pwd -P); case "$(CARGO_TARGET_DIR)" in /*) target=$(CARGO_TARGET_DIR);; *) target=$$root/$(CARGO_TARGET_DIR);; esac; \
	[ -n "$$target" ] && [ "$$target" != / ] && [ "$$target" != "$$root" ] || { printf '%s\n' 'ERROR: refusing unsafe CARGO_TARGET_DIR.' >&2; exit 1; }; \
	case "$$target" in "$$root"/*|/tmp/*) ;; *) printf '%s\n' 'ERROR: CARGO_TARGET_DIR must be repository-local or under /tmp.' >&2; exit 1 ;; esac; \
	prefix="$(PREFIX)"; case "$$prefix" in /*) ;; *) printf '%s\n' 'ERROR: PREFIX must be an absolute path.' >&2; exit 1 ;; esac; \
	case "$$prefix" in *'/../'*|../*|*/..|..) printf '%s\n' 'ERROR: PREFIX must not contain parent-directory traversal.' >&2; exit 1 ;; esac; \
	[ "$$prefix" != / ] && [ "$$prefix" != /.local ] && [ "$$prefix" != "$$root" ] || { printf '%s\n' 'ERROR: refusing unsafe PREFIX (filesystem or repository root).' >&2; exit 1; }; \
	reject_symlink_chain() { path=$$1; rest=$${path#/}; current=/; while [ -n "$$rest" ]; do component=$${rest%%/*}; [ "$$rest" = "$$component" ] && rest= || rest=$${rest#*/}; current="$$current$$component"; [ ! -L "$$current" ] || { printf '%s\n' "ERROR: refusing symlink in PREFIX path: $$current" >&2; exit 1; }; [ ! -e "$$current" ] || [ -d "$$current" ] || { printf '%s\n' "ERROR: PREFIX path component is not a directory: $$current" >&2; exit 1; }; current="$$current/"; done; }; \
	reject_symlink_chain "$$prefix"; \
	if [ -f "$$prefix/.asb-make-staging" ]; then reject_symlink_chain "$$prefix/bin"; rm -f -- "$$prefix/bin/asb" "$$prefix/.asb-make-staging"; rmdir -- "$$prefix/bin" 2>/dev/null || :; fi; \
	if [ "$$target" != "$$root/target" ] || [ -d "$$target" ]; then rm -rf -- "$$target"; fi

update: check-deps
	@set -eu; \
	"$(GIT)" diff --quiet && "$(GIT)" diff --cached --quiet || { printf '%s\n' 'ERROR: update requires a clean source tree (commit or stash changes first).' >&2; exit 1; }; \
	"$(GIT)" pull --ff-only; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" metadata --locked --no-deps >/dev/null || { \
		printf '%s\n' 'ERROR: Cargo.lock is not valid for the workspace; run make refresh-lock deliberately, review Cargo.lock, then commit it.' >&2; exit 1; \
	}

refresh-lock: check-deps
	@set -eu; \
	"$(GIT)" diff --quiet && "$(GIT)" diff --cached --quiet || { printf '%s\n' 'ERROR: refresh-lock requires a clean source tree (commit or stash changes first).' >&2; exit 1; }; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" update; \
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" metadata --locked --no-deps >/dev/null || { \
		printf '%s\n' 'ERROR: refreshed Cargo.lock failed locked validation; inspect and repair Cargo.lock before committing.' >&2; exit 1; \
	}; \
	printf '%s\n' 'Cargo.lock refreshed. Review the diff and commit it intentionally; locked build/test/install remain separate.'

test: check-deps
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" fmt --all -- --check
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" clippy --locked --workspace --all-targets --target "$(TARGET)" -- -D warnings
	@CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" test --locked --workspace --target "$(TARGET)"
	@RUSTDOCFLAGS='-D warnings' CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" "$(CARGO)" +"$(RUSTUP_TOOLCHAIN)" doc --locked --workspace --no-deps --target "$(TARGET)"
