.PHONY: all build check test run fmt clippy clean icons build-arch build-flatpak install uninstall

PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
CARGO_TARGET_DIR ?= $(shell cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
ifeq ($(CARGO_TARGET_DIR),)
CARGO_TARGET_DIR := target
endif
RELEASE_BIN ?= $(CARGO_TARGET_DIR)/release/uninote

all: build

build:
	cargo build --workspace

release:
	cargo build --release -p uninote-app

check:
	cargo check --workspace

test:
	cargo test --workspace

run:
	cargo run -p uninote-app

fmt:
	cargo fmt --all --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

icons:
	python3 scripts/generate_app_icons.py

build-arch: icons
	bash scripts/build_arch_package.sh

build-flatpak: icons
	bash scripts/build_flatpak_bundle.sh

install: release icons
	@echo "Installazione di UniNote in $(PREFIX)..."
	install -Dm755 $(RELEASE_BIN) $(DESTDIR)$(BINDIR)/uninote
	install -Dm644 data/org.uninote.UniNote.desktop $(DESTDIR)$(DATADIR)/applications/org.uninote.UniNote.desktop
	install -Dm644 data/org.uninote.UniNote.metainfo.xml $(DESTDIR)$(DATADIR)/metainfo/org.uninote.UniNote.metainfo.xml
	install -Dm644 data/org.uninote.UniNote.xml $(DESTDIR)$(DATADIR)/mime/packages/org.uninote.UniNote.xml
	install -Dm644 data/icons/hicolor/scalable/apps/org.uninote.UniNote.svg $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/org.uninote.UniNote.svg
	install -Dm644 data/icons/hicolor/symbolic/apps/org.uninote.UniNote-symbolic.svg $(DESTDIR)$(DATADIR)/icons/hicolor/symbolic/apps/org.uninote.UniNote-symbolic.svg
	install -Dm644 data/icons/hicolor/scalable/mimetypes/application-x-uninote.svg $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/mimetypes/application-x-uninote.svg
	@for res in 16 32 48 64 128 256 512; do \
		if [ -f data/icons/hicolor/$${res}x$${res}/apps/org.uninote.UniNote.png ]; then \
			install -Dm644 data/icons/hicolor/$${res}x$${res}/apps/org.uninote.UniNote.png $(DESTDIR)$(DATADIR)/icons/hicolor/$${res}x$${res}/apps/org.uninote.UniNote.png; \
		fi; \
	done
	@for res in 48 128; do \
		if [ -f data/icons/hicolor/$${res}x$${res}/mimetypes/application-x-uninote.png ]; then \
			install -Dm644 data/icons/hicolor/$${res}x$${res}/mimetypes/application-x-uninote.png $(DESTDIR)$(DATADIR)/icons/hicolor/$${res}x$${res}/mimetypes/application-x-uninote.png; \
		fi; \
	done
	@if command -v update-desktop-database >/dev/null 2>&1; then \
		update-desktop-database $(DESTDIR)$(DATADIR)/applications || true; \
	fi
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -q -t -f $(DESTDIR)$(DATADIR)/icons/hicolor || true; \
	fi
	@if command -v update-mime-database >/dev/null 2>&1; then \
		update-mime-database $(DESTDIR)$(DATADIR)/mime || true; \
	fi
	@echo "UniNote installato con successo!"

uninstall:
	@echo "Disinstallazione di UniNote da $(PREFIX)..."
	rm -f $(DESTDIR)$(BINDIR)/uninote
	rm -f $(DESTDIR)$(DATADIR)/applications/org.uninote.UniNote.desktop
	rm -f $(DESTDIR)$(DATADIR)/metainfo/org.uninote.UniNote.metainfo.xml
	rm -f $(DESTDIR)$(DATADIR)/mime/packages/org.uninote.UniNote.xml
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/org.uninote.UniNote.svg
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/symbolic/apps/org.uninote.UniNote-symbolic.svg
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/mimetypes/application-x-uninote.svg
	@for res in 16 32 48 64 128 256 512; do \
		rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/$${res}x$${res}/apps/org.uninote.UniNote.png; \
	done
	@for res in 48 128; do \
		rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/$${res}x$${res}/mimetypes/application-x-uninote.png; \
	done
	@echo "UniNote rimosso con successo!"

clean:
	cargo clean
	rm -rf build/ dist/
