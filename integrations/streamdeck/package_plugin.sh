#!/bin/bash
set -e

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"
OUT_DIR="$DIR/../../dist-streamdeck"

mkdir -p "$OUT_DIR"

echo "1. Generating action icons..."
python3 "$DIR/generate_icons.py"

echo "2. Packaging .streamDeckPlugin archives..."
rm -f "$OUT_DIR/TrackHelm.streamDeckPlugin"
rm -f "$OUT_DIR/com.trackhelm.controller.streamDeckPlugin"

cd "$DIR"
zip -r "$OUT_DIR/com.trackhelm.controller.streamDeckPlugin" com.trackhelm.controller.sdPlugin/
cp "$OUT_DIR/com.trackhelm.controller.streamDeckPlugin" "$OUT_DIR/TrackHelm.streamDeckPlugin"

echo "✓ Created: dist-streamdeck/TrackHelm.streamDeckPlugin"
