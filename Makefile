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
SPLICE_PACKAGES := $(notdir $(patsubst %/Cargo.toml,%,$(wildcard crates/*/Cargo.toml))) gate
MAC_CRATES ?= $(filter apple_sys splice_media splice_av splice_capture splice_app,$(SPLICE_PACKAGES))

MAC      ?= snowbear@olegs-mac-mini
MAC_DIR  ?= projects/editor
# The Mini is shared with other builds: low priority, two jobs.
MAC_ENV   = cd $(MAC_DIR) && export CARGO_BUILD_JOBS=2 PATH="$$HOME/.cargo/bin:$$PATH" &&
MAC_NICE  = nice -n 10

.PHONY: check build lib dev install bundle sign dmg mac-sync mac-check mac-local-check mac-run mac-pull eval loop

check:
	cargo run -q -p gate -- all
	cargo fmt $(foreach crate,$(SPLICE_PACKAGES),-p $(crate)) --check
	cargo clippy -q --workspace --all-targets -- -D warnings
	cargo t --workspace

ifeq ($(OS),Darwin)
export SDL3_DIR := $(abspath target/sdl-macos)
.PHONY: sdl
lib: sdl
sdl:
	packaging/build-sdl-macos.sh '$(SDL3_DIR)'
endif

lib:
	@test -e pxui/Cargo.toml || git submodule update --init pxui
ifeq ($(OS),Darwin)
	cargo build $(CARGOFLAGS) --manifest-path pxui/Cargo.toml -p pxui_dylib --target-dir target
	ln -sfn libpxui.dylib $(PRODUCTS)/libpxui.1.dylib
	ln -sfn '$(SDL3_DIR)/lib/libSDL3.0.dylib' $(PRODUCTS)/libSDL3.0.dylib
endif

build: lib
	cargo build $(CARGOFLAGS) -p splice_cli

ifeq ($(OS),Darwin)
SPLICE_SIGNING_KEYCHAIN ?= $(HOME)/Library/Keychains/splice-signing.keychain-db
BUNDLE_DIR ?= target/bundle

bundle: build
	@/usr/bin/security unlock-keychain -p '' '$(SPLICE_SIGNING_KEYCHAIN)' >/dev/null 2>&1 || \
		{ printf '%s\n' 'Set up the passwordless splice-signing keychain with the Developer ID Application identity for team 7ZH3FYV964.' >&2; exit 1; }
	packaging/bundle-macos.sh '$(PRODUCTS)' '$(SPLICE_SIGNING_KEYCHAIN)' '$(SDL3_DIR)' '$(BUNDLE_DIR)' '$(BUNDLE_MODE)'

sign: bundle

# T7.6: signed DMG in target/dist; NOTARY_PROFILE=<notarytool profile> also notarizes and staples.
dmg: bundle
	@/usr/bin/security unlock-keychain -p '' '$(SPLICE_SIGNING_KEYCHAIN)' >/dev/null 2>&1 || true
	tools/notarize-macos.sh '$(BUNDLE_DIR)/Splice.app' '$(SPLICE_SIGNING_KEYCHAIN)' target/dist

install:
	@mkdir -p '$(APPDIR)' '$(PREFIX)/bin'
	@test ! -e '$(PREFIX)/bin/splice' || test -L '$(PREFIX)/bin/splice' || \
		{ printf '%s\n' 'Move the existing $(PREFIX)/bin/splice before installing.' >&2; exit 1; }
	$(MAKE) bundle BUNDLE_DIR='$(APPDIR)' BUNDLE_MODE=--install
	appdir=$$(cd '$(APPDIR)' && pwd -P) && ln -sfn "$$appdir/Splice.app/Contents/MacOS/splice" '$(PREFIX)/bin/splice'
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
	@{ git rev-parse --short=9 HEAD | tr -d '\n'; git diff --quiet HEAD -- || printf -- '-dirty'; } > .build-sha
	rsync -az --delete --exclude /target --exclude /.git --exclude '.git' --exclude '*.splice/cache' --exclude /.claude ./ $(MAC):$(MAC_DIR)/

mac-check: mac-sync
	ssh $(MAC) '$(MAC_ENV) $(MAC_NICE) make mac-local-check'

# Runs on the Mini only (invoked by mac-check).
mac-local-check:
	/usr/bin/plutil -lint packaging/Info.plist packaging/splice.entitlements
ifneq ($(filter splice_app,$(MAC_CRATES)),)
	$(MAKE) lib PROFILE=debug
endif
ifneq ($(strip $(MAC_CRATES)),)
	cargo t $(foreach crate,$(MAC_CRATES),-p $(crate))
endif
ifneq ($(filter splice_app,$(MAC_CRATES)),)
	# The whole MCP and CLI suites: their macOS paths (export, frames, capture, music
	# measurement) only run here, and filtered runs let failures rot unnoticed.
	cargo t -p splice_mcp -p splice_cli
	# CLI export end to end (chapters, H.264/HEVC, frame comparison, cancel): it hung for
	# weeks unnoticed while it ran outside the gate.
	sh tools/check-macos-export.sh
endif
	SPLICE_TIMESTAMP=--timestamp=none $(MAKE) bundle PROFILE=debug
	tools/check-macos-bundle.sh '$(BUNDLE_DIR)/Splice.app'
	sh tools/test-macos-bundle.sh '$(BUNDLE_DIR)/Splice.app'
	sh tools/check-macos-open.sh '$(BUNDLE_DIR)/Splice.app'

mac-run: mac-sync
	ssh $(MAC) '$(MAC_ENV) $(MAC_NICE) $(CMD)'

# Live recording through the installed app (T22.1; SOAK=1800 for the T22.6 soak). Records the screen.
mac-record-check: mac-sync
	ssh $(MAC) '$(MAC_ENV) $(MAC_NICE) make install >/dev/null && python3 tools/record-check.py $(if $(SOAK),--soak $(SOAK))'

mac-pull:
	mkdir -p target/mac && rsync -az $(MAC):$(F) target/mac/

eval:
	tools/eval.sh

loop:
	tools/loop.sh
