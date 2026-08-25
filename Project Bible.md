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

### 2.9 Instant Waveform Rendering & Deep Zoom Architecture
* **Decision:** Multi-resolution precomputed single-sample peak pyramid with on-demand sample cache.
* **Rationale:** Avoids transmitting massive raw float payloads over the IPC bridge while maintaining extreme rendering precision.
* **Implementation Details:**
  * `overview_peaks`: 1,000 floats for full-file overview.
  * `pyramid_peaks`: 32,768 floats for responsive intermediate zooming.
  * `localSampleCache`: Small synchronous slice buffer fetching raw samples dynamically only when zoomed into $< 400$ individual samples.
  * **Continuous Unbroken Oscillating Line:** Single continuous line rendering at all zoom levels, eliminating sawtooth min/max ramp artifacts.
  * **Progressive Sample Node Squares (RX Style):** Individual sample node squares adaptively scale from $2.5\text{px}$ up to $6.0\text{px}$ bordered boxes as the display zooms in to the 9-sample maximum limit.
  * **Smart Playhead Centering:** Playhead locks to center screen when zoomed in for continuous tracking, but smoothly sweeps edge-to-edge when zoomed out to $1\times$.

### 2.10 Zero-Delay File Loading & Background Prefetch Architecture
* **Decision:** Stream buffer expansion, SIMD native build flags, and asynchronous background pre-decoding.
* **Rationale:** Reading multi-minute uncompressed audio files can cause perceptible UI delays (~1s) if decoded synchronously on double-click.
* **Implementation Details:**
  * **Expanded Stream Buffers:** `MediaSourceStreamOptions { buffer_len: 128 * 1024 }` and preallocated vector capacity in `trackhelm-engine/src/decoder.rs`.
  * **SIMD Native Optimization:** `[profile.dev.package."*"] opt-level = 3` in root `Cargo.toml` ensures all DSP and decoders compile with full Apple Silicon NEON vectorization even in development builds.
  * **Single-Click Background Prefetching (`preload_track`):** Single-clicking or navigating to an audio file in the file browser or playlist dispatches a non-blocking background decode into an in-memory `track_cache`. By the time the user double-clicks or presses Space, playback starts in **$< 1\text{ms}$**.

### 2.11 Per-Track Persistent Project Profiles (AnyTune Style)
* **Decision:** Automatic per-file state serialization (`TrackProfile`) keyed by canonical file path.
* **Rationale:** Musicians expect all adjustments made during rehearsal (volume, tempo, pitch, markers, attached chord charts, and alternate takes) to be instantly restored when returning to that song.
* **Preserved Parameters:**
  * `dbVolume`, `speed`, `pitch`
  * `eqBass`, `eqTreble`
  * `compressorThreshold`, `compressorRatio`, `compressorMakeup`
  * `markers` (IDs, names, timestamps, and colors)
  * `pdfChartPath`, `pdfChartName`
  * `associatedVersions` (alternate backing tracks, stems, and live takes)
  * `alternateTrackPath`
* **Lifecycle:** When switching tracks, the outgoing track's profile is automatically persisted to disk, and the incoming track's profile is restored and dispatched directly to the real-time DSP engine.

### 2.12 Markers & Rehearsal Regions System
* **Decision:** Interactive, color-coded, draggable markers with synchronized multi-view feedback.
* **Features:**
  * **Vibrant Rehearsal Palette:** Amber/Orange (Default/Intro), Cyan/Blue (Verse), Green (Solo), Purple (Chorus), Red (Bridge/Outro), Yellow (Cues/Loops).
  * **Inline Renaming:** Double-clicking any marker in the sidebar enables inline text editing with instant validation (`Enter` saves, `Esc` cancels).
  * **Waveform & Ruler Synchronization:** Renders matching colored flags in the top time ruler and vertical locator lines through the waveform.
  * **Navigation Shortcuts:** `ArrowLeft` and `ArrowRight` jump to previous and next markers during playback.

### 2.13 Rehearsal Workstation Knob & Hardware Reference Controls
* **Decision:** Custom analog-style virtual knobs with vertical drag sensitivity and hardware zero-point reference marks.
* **Features:**
  * **Zero/Center Ticks:** Top 12 o'clock center tick marks (`.knob-zero-tick`) on all parameter dials (Speed, Pitch, Fine Tune, Bass, Mid, Treble, Threshold, Ratio, Makeup) for precise visual alignment.
  * **Double-Click Reset:** Double-clicking any knob or slider instantly resets it to its unity default value.
  * **Real-time DSP Dispatch:** Live knob movements dispatch parameter updates to `SignalsmithStretch` on the native audio thread with zero stutter.

### 2.14 Full Audio Tag Metadata Inspector & In-Place Editor (Lofty Integration)
* **Decision:** Integrated native audio tag reading and non-destructive writing via `lofty-rs` (v0.21) across ID3v1/v2, Vorbis Comments, MP4/M4A atoms, and FLAC/RIFF chunks.
* **Editable Tag Fields:** Song Title, Artist/Performer, Album/Project, Grouping (Movement/Act/Scene/Band), Composer/Arranger, Genre, Year, Track Number, and Comments/Notes.
* **Inspector:** Dedicated `ℹ️ METADATA` split-view tab in the Rehearsal Deck displaying editable tags on the left and technical stream specs (sample rate, channels, duration, file path) on the right.

### 2.15 Dual-Mode Stable Waveform Rendering (Peak Bars vs. Sample Interpolation)
* **Decision:** Dual-phase waveform visualization combining solid peak-holding column bars when zoomed out with continuous sample interpolation and RX-style node squares when zoomed in.
* **Acoustic Rationale:** Single-point subsampling of high-frequency audio oscillations during scrolling playback causes phase-aliasing jitter (visual "fluttering"). Calculating max absolute peak per pixel column eliminates flutter at all zoom levels (including the 15-second rehearsal view).
* **Sample-Level Transition:** When zoomed in to $< 400$ frames on screen, the display automatically transitions to the unbroken continuous sample line with draggable node points.

### 2.16 3-Knob Pitch & Time Shifting Interface (Speed, Semitones, Fine Tune Cents)
* **Decision:** 3-dial hardware-style row with dedicated Fine Tune ($\pm 100\text{ cents}$) adjustment, placing Semitone transposition in the middle and Speed on the left.
* **Formula:** $\text{Effective Semitones} = \text{Semitones} + \frac{\text{Cents}}{100.0}$.
* **Persistence:** Both coarse semitone shifts and fine-tune cent adjustments are saved to the persistent `TrackProfile` and updated in real-time on `SignalsmithStretch`.

### 2.17 Native Real-Time DSP Engine: 3-Band Biquad EQ & Feedforward Dynamic Compressor
* **Decision:** Real-time audio DSP filters compiled directly into `trackhelm-engine` and processed inside the CPAL stream rendering loop.
* **3-Band Parametric Equalizer (`biquad.rs`):**
  * Robert Bristow-Johnson (RBJ) Audio EQ Cookbook biquad cascade operating with 64-bit float precision.
  * **Low Shelf Filter:** $100\text{ Hz}$ center frequency ($\pm 12\text{ dB}$, $Q=0.707$).
  * **Parametric Bell Filter:** $1000\text{ Hz}$ center frequency ($\pm 12\text{ dB}$, $Q=0.707$).
  * **High Shelf Filter:** $8000\text{ Hz}$ center frequency ($\pm 12\text{ dB}$, $Q=0.707$).
  * **Zero-Overhead Bypass:** Stereo audio samples bypass biquad computation entirely when all gains are set to $0\text{ dB}$.
* **Feedforward Soft-Knee Compressor (`compressor.rs`):**
  * Soft knee ($3.0\text{ dB}$ transition width) with feedforward detection topology.
  * Log-domain decibel envelope follower with ballistic smoothing ($30\text{ ms}$ attack time, $300\text{ ms}$ adaptive smooth decay release).
  * Continuously adjustable ratio ($1.0:1$ up to $4.0:1$) and linear makeup gain ($0\text{ dB}$ to $+24\text{ dB}$).
  * Unity default state: $0\text{ dB}$ threshold, $1.0:1$ ratio, $0\text{ dB}$ makeup gain (zero coloration).

### 2.18 Timeline Regions, Dynamic Hotkeys, Loop/Vamp & Cut Engine
* **Decision:** Interactive time region framework layered over the timeline with non-destructive playback engine integration.
* **Interactive Time Selection & Edge Grab Handles:**
  * **Shift + Drag** on the waveform creates a highlighted time selection box with live grab pills ($\pm 8\text{px}$ hover detection with `ew-resize` cursor) allowing edge adjustments prior to committing.
  * **Shift + Click** two markers in the sidebar or along the top ruler forms an active time selection span between them.
  * Created regions feature draggable left and right boundary handles, updating start and end boundaries in real time with automatic engine synchronization.
* **Quick Workflow Hotkeys:**
  * **`R`**: Creates a standard region from the active selection or marker pair.
  * **`X` (Cut / Skip Mode ✂️)**: Creates a new region or toggles an existing region into Cut mode. In the waveform, cuts appear grayed-out with red diagonal hazard hatching; the native audio engine seamlessly jumps over the cut span during playback.
  * **`L` (Loop / Vamp Mode 🔁)**: Creates a new region or toggles an existing region into Loop mode. In the waveform, loops appear with green background highlighting and bracket flags; the native audio engine continuously and seamlessly wraps playback back to region start.
* **Sidebar Management & Renaming:**
  * Dedicated **REGIONS** section in the right sidebar below markers.
  * Inline renaming via double-click, dedicated `✏️` button, or right-click context menu (`✏️ Rename Region...`).

### 2.19 Waveform Visual Overlays: Compressor Dotted Threshold & Ghost Dynamics
* **Dotted Threshold Boundary:**
  * Lowering the compressor threshold below $0\text{ dB}$ renders upper and lower yellow dotted boundary lines across the waveform with numerical dB readouts.
* **Dual Ghost + Compressed Waveform Visualization:**
  * When dynamic compression is active (Threshold $< 0\text{ dB}$, Ratio $> 1.0:1$), the uncompressed track waveform remains visible as a subtle translucent white ghost, while the dynamically compressed waveform is rendered in full vibrant theme color.

### 2.20 Effects Modules UI & Advanced Inspectors
* **Vertical 90° Module Tab Buttons:**
  * Rotated blue tab buttons (`COMP` and `EQ`) mounted along the left edge of each rack module row.
* **Advanced Inspector Modals:**
  * **Advanced Compressor Inspector:** Features a dynamic compression transfer curve SVG graph, interactive gain reduction meter, and ballistic specifications.
  * **Advanced Parametric EQ Inspector:** Features a frequency response curve visualizer across $20\text{ Hz} - 20\text{ kHz}$ with interactive node indicators for $100\text{ Hz}$, $1\text{ kHz}$, and $8\text{ kHz}$.

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

## 5. Milestone Plan & Progress
* **Milestone 0:** Architecture Decisions & Workspace Skeleton *(Completed)*
* **Milestone 1:** Minimal Viable Playback Engine *(Completed — CPAL stream, dual waveform display, deep zoom, overview seek, zero-delay load)*
* **Milestone 2:** Pitch and Time Shifting *(Completed — Native Signalsmith Stretch real-time integration, speed, pitch, and fine-tune dials, bypass optimizations)*
* **Milestone 3:** Loops, Markers, and Vamp Mode *(Completed — Interactive color-coded markers, Shift+click multi-marker selection, timeline regions, loop/vamp mode with seamless audio engine wrapping, draggable edge handles, hotkeys L, R)*
* **Milestone 4:** Persistent Projects & Track Profiles *(Completed — Per-track persistent AnyTune-style profiles preserving DSP, markers, regions, PDF charts, metadata, and associated tracks)*
* **Milestone 5:** EQ and Compressor DSP *(Completed — 3-band RBJ biquad EQ, feedforward soft-knee compressor, vertical 90° module tab buttons, inspector dialogs, dotted threshold lines, ghost compressed waveform visualizer)*
* **Milestone 6:** Nonlinear Timeline (Cuts) and Volume Envelopes *(Partially Completed — Real-time audio engine cut-skip integration, hazard hatch visualizer, hotkey X, and edge resizing handles complete)*
* **Milestone 7:** Library Management, Associated Media, and Metadata *(Completed — Integrated OS folder browser, playlist management, PDF chart association, alternate audio takes, Lofty ID3/Vorbis tag editor)*
* **Milestone 8:** MIDI, OSC, and QLab Integration
* **Milestone 9:** Export Engine
* **Milestone 10:** Practice Mode / Rehearsal Sequences
* **Later:** Advanced UI features, Sync Anchors, VST/AU hosting, Command Palette.
