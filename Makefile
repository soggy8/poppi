PREFIX ?= $(HOME)/.local
BINDIR = $(PREFIX)/bin
APPDIR = $(PREFIX)/share/applications
ICONDIR = $(PREFIX)/share/icons/hicolor/scalable/apps

.PHONY: build test lint smoke install uninstall

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings
	cargo clippy --all-targets --features ui-test -- -D warnings

# Drives the real UI in a headless compositor (needs mutter).
smoke:
	scripts/ui-smoke.sh

install: build
	install -Dm755 target/release/poppi_launcher $(DESTDIR)$(BINDIR)/poppi_launcher
	install -Dm644 data/com.poppi.launcher.desktop $(DESTDIR)$(APPDIR)/com.poppi.launcher.desktop
	install -Dm644 data/com.poppi.launcher.svg $(DESTDIR)$(ICONDIR)/com.poppi.launcher.svg

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/poppi_launcher
	rm -f $(DESTDIR)$(APPDIR)/com.poppi.launcher.desktop
	rm -f $(DESTDIR)$(ICONDIR)/com.poppi.launcher.svg
