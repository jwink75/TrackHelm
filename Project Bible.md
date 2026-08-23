# TrackHelm Project Bible v1.0

TrackHelm is a professional music rehearsal and playback workstation with non-destructive audio editing capabilities layered on top of rehearsal tools.

---

## 1. Product Vision

The primary user story for TrackHelm is:
> I have a recording, and I want to manipulate it instantly so I can rehearse, learn, analyze, and perform with it.

It is **not** a DAW or a simple audio editor; it is a rehearsal tool optimized for fast, zero-latency playback, legibility in low-light stage environments, and flexible external control.

**Target Environment:** macOS-native dark-mode desktop application.

---

## 2. Core Architecture Decisions (Milestone 0 ADR)

### 2.1 Authoritative Audio Engine
* **Decision:** Native Rust real-time audio thread using `cpal` for low-latency CoreAudio backend integration.
* **Rationale:** Tauri's webview runs in a sandbox and cannot natively host third-party VST/AU plugins (a long-term project requirement). The UI will act as a thin client communicating with the native Rust engine.
* **Communication:** Lock-free, thread-safe ring buffers (e.g., `ringbuf` or `rtrb` crates) will handle audio data passing, and lightweight channel-based queues will route controls from the main thread.

### 2.2 Time/Pitch Shifting Engine
* **Decision:** Native C++ compilation of `Signalsmith Stretch` linked directly to the Rust engine via FFI bindings.
* **Rationale:** Avoids the latency and sandbox constraints of WASM-in-webview solutions. Compiling C++ natively as a static library during Rust build time (`cc` crate) provides optimal performance and integration.

### 2.3 Audio Decode Library
* **Decision:** `symphonia` (pure Rust decoder).
* **Rationale:** Supports WAV, AIFF, FLAC, MP3, AAC/M4A, and Ogg Vorbis natively without depending on external system dynamic link libraries. It provides stable, fast decoding across different sample rates, bit depths, and channel configurations.

### 2.4 Persistence Architecture
* **Decision:** Hybrid approach:
  * **Structured Data:** SQLite via `rusqlite` on the Rust backend. Stores projects, playlists, markers, notes, associations, and MIDI/OSC mappings.
  * **Derived Binary Cache (Waveform Peaks):** Multi-resolution min/max decibel values stored as flat binary `.peak` files in the user cache directory (`~/Library/Caches/TrackHelm/peaks/`). Files are indexed by fingerprint/hash of the media to survive file renames and moves.

### 2.5 Plugin Hosting Feasibility Path
* **Decision:** The audio engine will process audio via a modular node-based routing graph (`AudioGraph`).
* **Implementation Plan:** Each source reader, DSP effect (EQ/compressor), and hosted plugin will implement a common `AudioNode` trait. For VST3, we will interface using `vst3-sys`. For Audio Units (AU), we will use macOS CoreAudio frameworks (`core-audio-rs`).

### 2.6 OSC/MIDI Library and Command Bus
* **Decision:** Command-pattern-based internal message bus.
* **Libraries:** `midir` for MIDI input/output; `rosc` for UDP-based OSC input/output.
* **Routing:** Incoming physical inputs (MIDI/OSC), keyboard shortcuts, and Tauri UI events are parsed into a single enum `enum Command` (e.g., `Play`, `Pause`, `Seek(Duration)`, `SetPitch(f32)`) and dispatched to the audio thread via a non-blocking command channel. Status updates are broadcasted back.

### 2.7 PDF Rendering Approach
* **Decision:** Embedded PDF viewer in the Tauri webview using `pdfjs-dist` (Mozilla PDF.js).
* **Rationale:** Keeps the heavy rendering visual workload in the frontend webview (rendering directly to HTML5 Canvas) without bloating the Rust backend. Allows easy canvas layering for annotations.

### 2.8 macOS App Bundle Packaging
* **Decision:** Tauri's built-in bundler will generate a standard macOS `.app` bundle.
* **Convenience:** A Makefile target `make app` will trigger the build and link the generated `TrackHelm.app` to the root workspace folder so it can be dragged to the macOS Dock for testing.

---

## 3. Core Requirements (MVP Scope)

### 3.1 Audio Engine & DSP
* Multi-format playback (WAV, AIFF, FLAC, MP3, AAC/M4A, Ogg Vorbis).
* Independent pitch shift and time stretch controls.
* Parametric EQ with interactive nodes + global bass/treble shelf controls.
* Feedforward compressor with dynamic waveform gain-reduction overlay.
* Non-destructive cuts (nonlinear playback timeline: jump from Time A to Time B with zero-crossing/crossfades).
* Node-based volume envelope drawn on waveform.
* Live layering (primary track + secondary reference pitches/clicks).
* BPM and key detection.
* Grand undo/redo, auto-save, and crash recovery.

### 3.2 Waveform & Timeline
* Dual waveform display: Large scrollable/zoomable waveform + mini Overview bar.
* Persistent peak cache.
* Color-coded, draggable, and renamable markers.
* Multiple concurrent loops with a "Vamp Mode" option (continuous loop until disabled).

### 3.3 Library, Projects & Files
* **Media vs. Project model:** Media is raw audio. Project stores playback state, loops, markers, EQ settings, associated assets, and notes. One Media file can have multiple independent Projects.
* Smart folders/playlists, reorderable virtual playlists with CSV import/export.
* Type-ahead search with 500ms debounce.
* **Associated Media System:** Link audio to alternate backing tracks, score PDFs, lyrics, video, or notes. Backing tracks and original recordings are hot-swappable during playback.
* Orphan detection using media hash/fingerprint.
* Preserved metadata tags with dedicated lyrics and application notes.

### 3.4 Hardware & External Control
* MIDI Learn interface (sending and receiving MIDI).
* Extensible OSC routing/mapping system with presets for QLab integration.
* Outgoing status events for external cue setups.

### 3.5 Export
* Non-destructive render of the timeline including EQ, compression, cuts, pitch, and tempo adjustments.
* Formats: WAV, M4A, MP3. Warning checks before overwriting original files.

---

## 4. Phase 2 & Later (Deferred - Do Not Build Yet)
* Elastic Alignment / Sync Anchors (piecewise time-stretch).
* Practice Mode / Rehearsal Sequences (loop chaining with tempo ramps).
* Command Palette (`Cmd+K`) and custom keyboard shortcuts.
* A/B processing state comparison.
* Output metering (LUFS/RMS meters).
* Full VST/AU plugin hosting implementation (infrastructure stub only in MVP).

---

## 5. Milestone Plan

* **Milestone 0:** Architecture Decisions & Workspace Skeleton (Current)
* **Milestone 1:** Minimal Viable Playback Engine (loading, cpal output, waveform, seek, volume)
* **Milestone 2:** Pitch and Time Shifting (Signalsmith integration)
* **Milestone 3:** Loops, Markers, and Vamp Mode
* **Milestone 4:** Persistent Projects & SQLite Database
* **Milestone 5:** EQ and Compressor DSP
* **Milestone 6:** Nonlinear Timeline (Cuts) and Volume Envelopes
* **Milestone 7:** Library Management, Associated Media, and Metadata
* **Milestone 8:** MIDI, OSC, and QLab Integration
* **Milestone 9:** Export Engine
* **Milestone 10:** Practice Mode / Rehearsal Sequences
* **Later:** Advanced UI features, Sync Anchors, VST/AU hosting, Command Palette.
