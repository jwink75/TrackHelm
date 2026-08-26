.PHONY: dev build app clean

dev:
	npm run tauri dev

build:
	npm run tauri build

app:
	npm run tauri build
	mkdir -p build
	rm -rf build/TrackHelm.app
	cp -R src-tauri/target/release/bundle/macos/TrackHelm.app build/
	@echo "=============================================="
	@echo "Success! TrackHelm.app bundle copied to:"
	@echo "  build/TrackHelm.app"
	@echo "You can now drag this app bundle to your macOS Dock."
	@echo "=============================================="

streamdeck:
	bash integrations/streamdeck/package_plugin.sh

clean:
	cargo clean
	rm -rf dist build dist-streamdeck
