# Splice. See AGENTS.md, PLAN.md and docs/LOOP.md.
#   make dev       debug profile (fast incremental) -> installed `splice` (macOS: signed Splice.app + symlink)
#   make install   release profile, same install
#   make check     Linux gate: policy gate, fmt, clippy, cargo t
#   make mac-*     drive the shared Mac mini over ssh, sparingly (docs/LOOP.md §3)
#   make loop      run the build loop (Codex by default; AGENT=claude for Claude Code)
PROFILE ?= release
PREFIX  ?= $(HOME)/.local
# One fixed install location: privacy grants follow bundle ID + signature, and capture runs only from here.
APPDIR  ?= $(HOME)/Applications
CARGOFLAGS = $(if $(filter release,$(PROFILE)),--release)
PRODUCTS   = target/$(if $(filter release,$(PROFILE)),release,debug)
OS := $(shell uname -s)

MAC      ?= snowbear@olegs-mac-mini
MAC_DIR  ?= projects/editor
# The Mini is shared with other builds: low priority, two jobs.
MAC_ENV   = cd $(MAC_DIR) && export CARGO_BUILD_JOBS=2 PATH="$$HOME/.cargo/bin:$$PATH" &&
MAC_NICE  = nice -n 10

.PHONY: check build lib dev install bundle sign mac-sync mac-check mac-local-check mac-run mac-pull eval loop

check:
	cargo run -q -p gate -- all
	cargo fmt --all --check
	cargo clippy -q --workspace --all-targets -- -D warnings
	cargo t --workspace

lib:
	@test -e pxui/Cargo.toml || git submodule update --init pxui
ifeq ($(OS),Darwin)
	cargo build $(CARGOFLAGS) --manifest-path pxui/Cargo.toml -p pxui_dylib --target-dir target
	ln -sfn libpxui.dylib $(PRODUCTS)/libpxui.1.dylib
endif

build: lib
	cargo build $(CARGOFLAGS) -p splice_cli

ifeq ($(OS),Darwin)
bundle: build
	packaging/bundle-macos.sh '$(PRODUCTS)'

sign: build
	tools/sign-macos.sh '$(PRODUCTS)'

install: bundle
	mkdir -p '$(APPDIR)' '$(PREFIX)/bin'
	rm -rf '$(APPDIR)/Splice.app' && ditto target/bundle/Splice.app '$(APPDIR)/Splice.app'
	ln -sfn '$(APPDIR)/Splice.app/Contents/MacOS/splice' '$(PREFIX)/bin/splice'
	/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f '$(APPDIR)/Splice.app'
	'$(PREFIX)/bin/splice' version
else
install: build
	install -d '$(PREFIX)/bin'
	install $(PRODUCTS)/splice '$(PREFIX)/bin/splice'
	'$(PREFIX)/bin/splice' version
endif

dev:
	$(MAKE) install PROFILE=debug

# ---- Mac mini (docs/LOOP.md §3): sync once, check once. ----
mac-sync:
	rsync -az --delete --exclude /target --exclude /.git --exclude '.git' --exclude '*.splice/cache' ./ $(MAC):$(MAC_DIR)/

mac-check: mac-sync
	ssh $(MAC) '$(MAC_ENV) $(MAC_NICE) make mac-local-check'

# Runs on the Mini only (invoked by mac-check).
mac-local-check:
	cargo t $(if $(MAC_CRATES),$(foreach c,$(MAC_CRATES),-p $(c)),--workspace)
	@if [ -x packaging/bundle-macos.sh ]; then $(MAKE) dev && tools/check-macos-bundle.sh $(APPDIR)/Splice.app; else echo 'mac-check: bundle not built yet (T7)'; fi

mac-run: mac-sync
	ssh $(MAC) '$(MAC_ENV) $(MAC_NICE) $(CMD)'

mac-pull:
	mkdir -p target/mac && rsync -az $(MAC):$(F) target/mac/

eval:
	tools/eval.sh

loop:
	tools/loop.sh
