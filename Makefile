PREFIX ?= $(HOME)/.local
BINDIR = $(PREFIX)/bin
APPDIR = $(PREFIX)/share/applications
ICONDIR = $(PREFIX)/share/icons/hicolor/scalable/apps
SYSTEMDUSERDIR = $(PREFIX)/share/systemd/user
# e.g. `make FEATURES=layer-shell install` for Sway, Hyprland, Niri or KDE.
FEATURES ?=

.PHONY: build test lint smoke install uninstall

build:
	cargo build --release $(if $(FEATURES),--features $(FEATURES))

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
	install -d $(DESTDIR)$(SYSTEMDUSERDIR)
	sed "s|@BINDIR@|$(BINDIR)|g" data/poppi-launcher.service > $(DESTDIR)$(SYSTEMDUSERDIR)/poppi-launcher.service

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/poppi_launcher
	rm -f $(DESTDIR)$(APPDIR)/com.poppi.launcher.desktop
	rm -f $(DESTDIR)$(ICONDIR)/com.poppi.launcher.svg
	rm -f $(DESTDIR)$(SYSTEMDUSERDIR)/poppi-launcher.service
