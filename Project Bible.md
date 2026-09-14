# TrackHelm Project Bible v1.0

TrackHelm is a professional music rehearsal, transcription, and playback workstation with non-destructive audio editing capabilities layered on top of rehearsal tools.

---

## 1. Product Vision

The primary user story for TrackHelm is:
> I have a recording, and I want to manipulate it instantly so I can rehearse, learn, analyze, and perform with it.

It is **not** a DAW or a simple audio editor; it is a rehearsal tool optimized for fast, zero-latency playback, legibility in low-light stage environments, and flexible external control.

**Target Environments:** Native cross-platform dark-mode desktop workstation supporting macOS (Apple Silicon & Intel) and Windows 10/11 (x64) with native installers (NSIS & MSI).

---

## 2. Core Architecture Decisions (ADRs)

### 2.1 Authoritative Audio Engine
* **Decision:** Native Rust real-time audio thread using `cpal` for low-latency backend integration (CoreAudio on macOS, WASAPI on Windows).
* **Rationale:** Tauri's webview runs in a sandbox and cannot natively host real-time DSP with deterministic low latency or third-party VST/AU plugins. The UI acts as a thin client communicating with the native Rust engine.
* **Communication:** Lock-free, thread-safe ring buffers handle audio data passing, and lightweight channel-based queues route controls from the main thread.

### 2.2 Time/Pitch Shifting Engine
* **Decision:** Native C++ compilation of `Signalsmith Stretch` linked directly to the Rust engine via FFI bindings.
* **Rationale:** Avoids the latency and sandbox constraints of WASM-in-webview solutions. Compiling C++ natively as a static library during Rust build time (`cc` crate) provides optimal performance and SIMD vectorization.

### 2.3 Audio Decode Library
* **Decision:** `symphonia` (pure Rust decoder).
* **Rationale:** Supports WAV, AIFF, FLAC, MP3, AAC/M4A, and Ogg Vorbis natively without depending on external system dynamic link libraries. It provides stable, fast decoding across different sample rates, bit depths, and channel configurations.

### 2.4 Persistence Architecture
* **Decision:** Hybrid approach:
  * **Structured Data:** SQLite via `rusqlite` on the Rust backend combined with debounced in-memory JSON profiles (`TrackProfile`) in the frontend. Stores projects, playlists, markers, notes, associations, and MIDI/OSC mappings.
  * **Derived Binary Cache (Waveform Peaks):** Multi-resolution min/max decibel values stored as flat binary `.peak` files in the user cache directory (`~/Library/Caches/TrackHelm/peaks/` on macOS, `%LOCALAPPDATA%\TrackHelm\peaks` on Windows). Files are indexed by fingerprint/hash of the media to survive file renames and moves.

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

### 2.8 macOS App Bundle & Packaging
* **Decision:** Tauri's built-in bundler generates a standard macOS `.app` bundle.
* **Convenience:** A Makefile target `make app` triggers the build and links the generated `TrackHelm.app` to the root workspace folder so it can be dragged to the macOS Dock for testing.

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

### 2.10 Zero-Delay File Loading & Bounded LRU Cache Architecture
* **Decision:** Stream buffer expansion, SIMD native build flags, bounded LRU cache (`LruTrackCache`), and asynchronous background pre-decoding.
* **Rationale:** Reading multi-minute uncompressed audio files can cause perceptible UI delays if decoded synchronously on double-click, while unbounded in-memory decoded PCM could consume gigabytes of RAM during long rehearsals.
* **Implementation Details:**
  * **Bounded LRU Cache:** Decoded audio is stored in an in-memory LRU cache (`LruTrackCache`) capped to active track + 5–6 recently used/preloaded songs (~500 MB max), automatically evicting oldest non-active tracks to guarantee bounded memory.
  * **Non-Blocking Asynchronous Load:** `load_track` and `preload_track` check the cache under a brief lock, decode on worker threads via `spawn_blocking` without holding the mutex, and insert back under a brief lock.
  * **Expanded Stream Buffers:** `MediaSourceStreamOptions { buffer_len: 128 * 1024 }` and preallocated vector capacity in `trackhelm-engine/src/decoder.rs` seeded from codec `n_frames` metadata.
  * **In-Flight Preload Deduplication:** Frontend tracks ongoing preload promises in a `Set<string>` to eliminate redundant duplicate decoding requests during rapid navigation.

### 2.11 Real-Time Audio Callback Safety & Parameter Coalescing
* **Decision:** Zero heap allocations on the CPAL real-time audio thread and atomic parameter coalescing.
* **Implementation Details:**
  * **Zero Allocation:** Scratch buffers (`in_channel_scratch`, `out_channel_scratch`) and channel slice pointer arrays are pre-allocated with fixed maximum capacities (16,384 frames), completely eliminating `Vec` and object allocations inside the real-time audio callback.
  * **In-Place Filter Pools:** A pre-allocated pool of 16 RBJ biquad filters is updated in-place on EQ parameter changes without vector dropping or re-allocation.
  * **Command Coalescing:** Drains the command receiver with discrete commands (Play, Pause, Seek, LoadAudio) executed sequentially, while continuous parameter updates (Pitch, Speed, EQ, Compressor, Volume, Regions) are coalesced into a single update per audio block.

### 2.12 Per-Track Persistent Project Profiles & Debounced Storage (AnyTune Style)
* **Decision:** In-memory profile store with debounced disk persistence (`TrackProfile`) keyed by canonical file path.
* **Implementation Details:**
  * **In-Memory Store:** The profile database is cached in memory (`cachedProfilesStore`), providing instant reads and writes without synchronous disk or `localStorage` serialization.
  * **Debounced Persistence:** Live parameter tweaks (knob dragging, sliders, text input) update memory instantly and debounce writes to storage by 400ms, flushing immediately on track change or window unload.
  * **Decoupled Waveform Redraws:** Playhead animation is gated on playback/scrubbing, while heavy waveform peak re-computations only occur on zoom, pan, window resize, or dirty flags.
* **Preserved Parameters:**
  * `dbVolume`, `speed`, `pitch`, `pitchCents`
  * `eqBass`, `eqTreble`, full `eqNodes` array
  * `compStage1`, `compStage2`, `compRouting`, `compParallelBlend`
  * `isEqBypassed`, `isCompressorBypassed`
  * `markers` (IDs, names, timestamps, colors, and PDF anchors)
  * `regions` (IDs, names, start/end times, loop/cut flags, crossfadeMs)
  * `primarySongTrackPath`: canonical root mix identifier
  * `pdfChartPath`, `pdfChartName`, `openPdfTabs`, active dynamic PDF state
  * `associatedVersions` (alternate backing tracks, stems, guide tracks)
  * `notesMarkdown`, `lyricsMarkdown`, markdown view modes

### 2.13 Markers & Rehearsal Regions System
* **Decision:** Interactive, color-coded, draggable markers with synchronized multi-view feedback.
* **Features:**
  * **Vibrant Rehearsal Palette:** Amber/Orange (Default/Intro), Cyan/Blue (Verse), Green (Solo), Purple (Chorus), Red (Bridge/Outro), Yellow (Cues/Loops).
  * **Inline Renaming:** Double-clicking any marker in the sidebar enables inline text editing with instant validation (`Enter` saves, `Esc` cancels).
  * **Waveform & Ruler Synchronization:** Renders matching colored flags in the top time ruler and vertical locator lines through the waveform.
  * **Navigation Shortcuts:** `ArrowLeft` and `ArrowRight` jump to previous and next markers during playback.

### 2.14 Rehearsal Workstation Knob & Hardware Reference Controls
* **Decision:** Custom analog-style virtual knobs with vertical drag sensitivity and hardware zero-point reference marks.
* **Features:**
  * **Zero/Center Ticks:** Top 12 o'clock center tick marks (`.knob-zero-tick`) on all parameter dials (Speed, Pitch, Fine Tune, Bass, Mid, Treble, Threshold, Ratio, Makeup) for precise visual alignment.
  * **Double-Click Reset:** Double-clicking any knob or slider instantly resets it to its unity default value.
  * **Real-time DSP Dispatch:** Live knob movements dispatch parameter updates to `SignalsmithStretch` on the native audio thread with zero stutter.

### 2.15 Full Audio Tag Metadata Inspector & In-Place Editor (Lofty Integration)
* **Decision:** Integrated native audio tag reading and non-destructive writing via `lofty-rs` (v0.21) across ID3v1/v2, Vorbis Comments, MP4/M4A atoms, and FLAC/RIFF chunks.
* **Editable Tag Fields:** Song Title, Artist/Performer, Album/Project, Grouping (Movement/Act/Scene/Band), Composer/Arranger, Genre, Year, Track Number, and Comments/Notes.
* **Inspector:** Dedicated `ℹ️ METADATA` split-view tab in the Rehearsal Deck displaying editable tags on the left and technical stream specs (sample rate, channels, duration, file path) on the right.

### 2.16 Timeline Regions, Dynamic Hotkeys, Loop/Vamp & Gapless Cut Engine
* **Decision:** Interactive time region framework layered over the timeline with non-destructive playback engine integration.
* **Interactive Time Selection & Edge Grab Handles:**
  * **Shift + Drag** on the waveform creates a highlighted time selection box with live grab pills ($\pm 8\text{px}$ hover detection with `ew-resize` cursor) allowing edge adjustments prior to committing.
  * **Shift + Click** two markers in the sidebar or along the top ruler forms an active time selection span between them.
  * Created regions feature draggable left and right boundary handles, updating start and end boundaries in real time with automatic engine synchronization.
* **Quick Workflow Hotkeys:**
  * **`R`**: Creates a standard region from the active selection or marker pair.
  * **`X` (Cut / Skip Mode ✕)**: Creates a new region or toggles an existing region into Cut mode. In the waveform, cuts appear grayed-out with red diagonal hazard hatching; the native audio engine seamlessly jumps over the cut span during playback with zero gap or silence latency (without resetting stretch latency buffers).
  * **`L` (Loop / Vamp Mode 🔁)**: Creates a new region or toggles an existing region into Loop mode. In the waveform, loops appear with green background highlighting and bracket flags; the native audio engine continuously and seamlessly wraps playback back to region start.
* **Splice Crossfade Control**: Direct click-to-edit badge (`✕ 5ms`) allows adjusting crossfade smoothing from $0$ to $100\text{ ms}$.

### 2.17 Advanced Dual-Stage Dynamic Compressor Console (Sonitus Inspired)
* **Dual Serial & Parallel Compressor Topology:**
  * Native CPAL DSP support for two independent compressor stages (`CompStage1` and `CompStage2`), switchable between **Series** (Stage 1 feeds into Stage 2) and **Parallel** routing with a continuous wet/dry blend slider.
* **4 Distinct Analog & Modern Character Models:**
  * **Vintage:** Tube-style soft compression with progressive non-linear saturation curve.
  * **Modern:** Transparent, ultra-clean VCA detection with linear transfer slope.
  * **FET:** Lightning-fast ballistic response ($0.1\text{ ms}$ attack, aggressive punch).
  * **Opto:** Smooth, musical electro-optical two-stage decay curve.
* **Exact Analytical Soft-Knee Transfer Function:**
  * Soft-knee geometry computed via continuous polynomial equation:
    $$y(x) = \begin{cases} x & x \le T - W/2 \\ x + \frac{(1/R - 1)(x - T + W/2)^2}{2W} & T - W/2 < x \le T + W/2 \\ T + \frac{x - T}{R} & x > T + W/2 \end{cases}$$
  * When $R = 1.0$, $(1/R - 1) = 0$, guaranteeing a pure linear diagonal reference line without vertical asymptotes.
* **Real-time Meters & Live Tracing:**
  * Dual L/R peak meters, fast-decay Gain Reduction (GR) meter, and animated green signal dot tracing input level along the curve in real time.

### 2.18 Advanced Precision Parametric Equalizer Console
* **Multi-Filter RBJ Biquad Cascade Engine:**
  * Real-time audio thread cascaded biquad vector supporting arbitrary numbers of simultaneous filter bands (Peaking Bell, Low Shelf, High Shelf, Low Pass, High Pass, Notch).
* **Interactive Node Dragging & Mousewheel Q Control:**
  * Direct click-and-drag on SVG filter nodes (horizontal logarithmic frequency $20\text{ Hz} - 20\text{ kHz}$, vertical gain $\pm 24\text{ dB}$).
  * Mouse wheel / scroll adjusts $Q$ bandwidth smoothly from $0.10$ to $10.00$.
  * Double-click on empty graph space or click `+ Add Filter Band` to drop a new node at the cursor position.
* **Logarithmically Mapped Frequency Sliders:**
  * Slider input normalized across $\log_{10}(f) \in [1.30103, 4.30103]$, ensuring equal distance per octave.
* **Real-Time Spectrum Analyzer (RTA):**
  * 64-band animated spiky FFT frequency spectrum glow responding dynamically to playback energy.

### 2.19 Elgato Stream Deck Plugin & Show Control Architecture (Milestone 8)
* **Dedicated Stream Deck Plugin (`com.trackhelm.controller.sdPlugin`)**:
  * 15-key and 32-key profiles with custom retina icons.
  * Dynamic centered LCD readouts updated at 10Hz:
    * Song name and live elapsed timestamp.
    * Current landmark marker name and cue timestamp.
    * Semitone pitch transposition (+1 / -1 st).
    * Calibrated volume in decibels (+1 / -1 dB).
    * Tempo speed multiplier (+5% / -5% speed).
    * Play/Pause indicator with live time.
    * Rewind and playlist progression (`Trk 2/8`).
  * Automated packaging and direct installation via `npm run build:streamdeck`.
* **Lag-Tolerant WebSocket Server (`ws://0.0.0.0:4545`)**:
  * Outbound broadcast channel using `tokio::sync::broadcast` with `RecvError::Lagged` tolerance, ensuring continuous uninterrupted streaming to connected remotes.
* **OSC & MIDI Infrastructure**:
  * UDP-based `/trackhelm/*` endpoints for QLab / Bitfocus Companion.
  * Hardware MIDI integration via `midir` (CC 7 Volume, CC 1 Speed, Note-on transport).

### 2.20 High-Fidelity Multi-Threaded Offline Audio Export Engine (Milestone 9)
* **Non-Destructive Audio Baking (`render_export_audio`)**:
  * Offline multi-threaded audio renderer processing the source PCM through the full DSP graph.
  * Bakes tempo stretch (Signalsmith), pitch shift, cascaded biquad EQ, dual-stage compression, and region cut splices into 16-bit, 24-bit, or 32-bit Float WAV files.
  * Configurable range export: Entire Song, Active Time Selection, or Selected Timeline Region.
  * Metadata preservation copying ID3v2/Vorbis tags and album artwork to exported files.

### 2.21 AI Stem Separation (UVR) & Safe File Deletion Architecture (Milestone 10)
* **Decision:** Headless execution of Ultimate Vocal Remover models via `python-audio-separator` running locally with hardware acceleration (Apple Silicon MPS on macOS, NVIDIA CUDA on Windows) combined with native AAC encoding (`afconvert` on macOS, `ffmpeg` on Windows/Linux) and OS-level Trash / Recycle Bin.
* **4-Model Ensemble Mode:** Kim Vocal 2 (`Kim_Vocal_2.onnx`), MDX23C-InstVoc HQ (`MDX23C-8KFFT-InstVoc_HQ.ckpt`), UVR-MDX-NET-Voc_FT (`UVR-MDX-NET-Voc_FT.onnx`), and Demucs v4 (`htdemucs_ft.yaml`) blended via `uvr_max_spec` to isolate master vocal stems.
* **Lead / Backing Vocals Separation:** VR Architecture `5_HP-Karaoke-UVR.pth` executed on isolated vocals to split lead vs. backing harmonies.
* **Hardware Acceleration (NVIDIA CUDA & Apple Silicon MPS):** On Windows, `scripts/uvr_separator.py` dynamically registers PyTorch's CUDA 12 runtime directory (`torch/lib`) via `os.add_dll_directory`, allowing both PyTorch models (MDX23C, Demucs) and ONNX models (Kim Vocal 2, UVR-MDX-NET via `onnxruntime-gpu`) to leverage NVIDIA GPUs (e.g. RTX 2070), reducing ensemble processing time from ~55 minutes on CPU to ~3–4 minutes.
* **Model Discovery & Automated Download Fallback:** TrackHelm scans for existing local Ultimate Vocal Remover installations (e.g. `%LOCALAPPDATA%\Programs\Ultimate Vocal Remover\models`) and links found models into `%LOCALAPPDATA%\TrackHelm\models_cache\` to avoid duplicate downloads. If UVR is not installed or specific models are missing, `audio-separator` automatically downloads the required weights on demand from HuggingFace/GitHub into the cache.
* **Self-Extracting Embedded Script Architecture:** Binary embeds `scripts/uvr_separator.py` as a failsafe using Rust `include_str!`. If running in packaged production environments without project roots, TrackHelm automatically unpacks the script to the user's local application data directory (`%LOCALAPPDATA%\TrackHelm\scripts\`), backed by Tauri `bundle.resources`.
* **Clean Stderr / Stdout Event Stream Filtering:** Python informational logs (`INFO`, `WARNING`, `tqdm` progress) are routed through stdout or filtered in the Rust host (`main.rs`). `stderr` is reserved exclusively for fatal exceptions and tracebacks, preventing benign status records from triggering error banners in the UI.
* **Windowless Background Process Execution (`CREATE_NO_WINDOW`):** Native Windows process creation flags (`0x08000000`) applied across Rust `Command` spawns and Python `subprocess.run` calls prevent any command prompt or terminal windows from flashing or popping up during stem separation.
* **Cross-Volume NTFS Junction Model Resolution:** Dynamic model cache linking transparently links pre-downloaded Ultimate Vocal Remover models across NTFS drive junctions without extra disk usage.
* **Audio Encoding & Organization:** Renders stems directly into `[OriginalDir]/Vocals Only/[Track] (Vocals Ensemble).m4a` with zero quality loss.
* **Safe OS Trash / Recycle Bin Deletion:** Associated files can be sent to macOS Trash or Windows Recycle Bin (`trash` crate) with user confirmation modal, preserving full native "Put Back" / Restore capabilities.

### 2.22 Bi-Directional Song Collections & Multi-Peer Store Synchronization (Milestone 11)
* **Decision:** True peer-to-peer song clusters rather than parent-child hierarchies.
* **Implementation Details:**
  * Storing `primarySongTrackPath` on each `TrackProfile`.
  * Opening any stem, guide track, or alternate take directly from the File Browser discovers existing peer profiles in storage, links all sibling audio files and PDF charts, and synchronizes markers, regions, and notes bidirectionally across all peer profiles.
  * Safe cleanup: Moving an audio track to Trash automatically prunes its path from all peer profiles.

### 2.23 Live Sample-Accurate A/B Compare Engine (`⇄ A/B` & Hotkey `T`) (Milestone 11)
* **Decision:** Dedicated A/B auditioning engine with zero-interruption playhead preservation.
* **Implementation Details:**
  * A dedicated `⇄ A/B` toggle button on the transport controls and global hotkey `T` flip between the designated default mix and any auditioned stem/alternate take.
  * Preserves exact playhead timestamp ($t$) during active playback, allowing immediate direct auditioning during rehearsals.
  * Dynamic watermark labels (`"VOCALS ONLY"`, `"LEAD VOCALS"`, `"BACKING VOCALS"`, `"HIGH-RES MASTER"`, `"ORIGINAL ARTIST"`, `"DEFAULT MIX"`) clearly identify the currently auditioned stream.

### 2.24 Streamlined Single-Line Files Hub & Active Track Action Pipeline (Milestone 11)
* **Decision:** Space-efficient single-line rows (~32px height) replacing bulky multi-line card tiles.
* **Implementation Details:**
  * Displays 10–12+ files without scrolling, displaying file icon, filename, format badge, role badges, and action buttons (`▶ Load`/`✓ Loaded`, UVR actions, `🗑️`, `×`).
  * **Active Track Always Visible:** The currently loaded track is always present in the files list with a glowing `● ACTIVE` badge and `✓ Loaded` state.
  * **Direct Stem Actions on Active Track:** Users can initiate `✨ Isolate Vocals` directly on whatever track is loaded (e.g. an original artist reference recording) without switching tracks.

### 2.25 Smooth Debounced Type-to-Jump Engine with Smart Name Stripping (Milestone 12)
* **Decision:** Immediate character & multi-character prefix matching with a 750ms typing window debounce.
* **Implementation Details:**
  * Supported across both File Browser and Playlist.
  * Typing the first character (e.g. `B`) immediately jumps and smoothly scrolls to the first matching song/folder.
  * Typing subsequent characters within the 750ms window (e.g. `E` -> `BE`) immediately jumps to the first matching `BE...` track (never treating `E` as an isolated keypress jumping to an "E" song).
  * Tapping the same key repeatedly cycles through all tracks starting with that character.
  * Intelligent regex stripping removes leading track numbering (`/^[0-9a-z]?\d+[\s\.\-_]+/`) and leading `The `, allowing intuitive matching.
  * Context-aware hotkey safety: DAW hotkeys (`M`, `R`, `X`, `L`, `T`) are preserved when focus is in the waveform area.

### 2.26 Setlist AI CSV Importer & Playlist Health Repair Engine (Milestone 12)
* **Decision:** Fuzzy setlist import with automated missing file and upgraded mix detection.
* **Implementation Details:**
  * Imports setlists from CSV spreadsheets and clipboard text.
  * Playlist Health & Repair (`🔄 Repair`) detects missing files and suggests upgraded lossless mixes with confidence scoring.
  * Preserves markers, regions, and rehearsal notes across relinked tracks.

### 2.27 Model Context Protocol (MCP) Server Integration (Milestone 13)
* **Decision:** Embedded MCP server interface over stdio / WebSocket.
* **Implementation Details:**
  * Exposes TrackHelm workstation control tools (`trackhelm_play`, `trackhelm_pause`, `trackhelm_seek`, `trackhelm_set_speed`, `trackhelm_set_pitch`, `trackhelm_add_marker`, `trackhelm_get_state`) directly to AI coding assistants and automation sidecars.

### 2.28 High-Fidelity Audio Resampling & Hardware Device Clock Synchronization (Milestone 14)
* **Decision:** Polyphase FFT sample rate resampling using `rubato 0.15` (`FftFixedInOut<f32>`) with real-time DSP clock synchronization.
* **Problem & Rationale:**
  * Modern audio interfaces and host OS sound subsystems (such as Windows WASAPI and macOS CoreAudio default output devices) typically run at **48,000 Hz**.
  * Audio tracks decoded at 44.1 kHz (CD standard, MP3, AAC) played into a 48 kHz DAC without conversion are consumed 8.84% too quickly, causing playback to speed up by $1.088\times$ and pitch to shift sharp by $+1.467$ semitones (~147 cents).
  * In addition, fixed-frequency DSP modules (such as Biquad EQ band cutoffs and compressor attack/release time constants) drift from their acoustical specifications if calculated using the file sample rate rather than the stream sample rate.
* **Implementation Details:**
  * **Background Pre-Resampling:** When loading tracks in `src-tauri`, audio is decoded and resampled in background worker threads to the device's native rate before entering the memory cache.
  * **Zero-Latency Output Delay Compensation:** Resampled buffers compensate for `output_delay()` and flush internal polyphase filter tails so audio timing and duration align to the exact microsecond.
  * **Bit-Transparent Passthrough:** When source and target sample rates match and modulation is disengaged (speed $= 1.0$, pitch $= 0.0$), the resampler is bypassed for pure bit-transparent fidelity.
  * **Engine Clock Synchronization:** `device_sample_rate` is queried from CPAL and stored in `SharedEngineState`. Signalsmith Stretch, cascaded Biquad EQ filters, and dual compressor envelopes are initialized and updated against `device_sample_rate`.
  * **Visual Waveform Alignment:** Waveform overview and pyramid peaks are calculated from the resampled buffer, ensuring absolute sample-accurate synchronization between the graphical playhead and the audible DAC output.

### 2.29 Cross-Platform Multi-Drive File System & Cloud Navigation (Milestone 15)
* **Decision:** Virtual "This PC" root directory and native cloud storage folder detection.
* **Implementation Details:**
  * **Windows Drive Enumeration:** On Windows, the File Browser provides a top-level virtual "This PC" / "Computer" root that queries available system volumes (`GetLogicalDrives`) and enumerates all mounted drive letters (`C:`, `D:`, `E:`, `F:`, etc.) with drive labels.
  * **Direct Cloud Folder Discovery:** Automatically discovers and bookmarks active cloud synchronization folders across user environments (Dropbox, Google Drive, Microsoft OneDrive, and Apple iCloud).

### 2.30 Height-Adjustable Waveform & Interactive Audio Tags (Milestone 16)
* **Decision:** Interactive height-resizable waveform workspace and customizable, auto-detected audio tag metadata.
* **Implementation Details:**
  * **Dynamic Waveform Resize Handle:** An interactive divider handle situated between the overview mini-waveform (40px) and the main detailed waveform allows smooth vertical resizing from 100px to 650px (default 256px). Includes double-click to reset to 256px and persists in `localStorage` (`th_waveform_height`).
  * **Auto-Detected & Context-Menu Toggleable Audio Tags:** Comprehensive tag system supporting automatic keyword detection (`"Original"`, `"Vocals only"`, `"Lead Vocal"`, `"Background Vocals"`, `"Track"`) and instant right-click toggleability across the `FILES` tab rows and active waveform context menu.
  * **Peer Collection Synchronization:** Tag metadata persists in `TrackProfile` (`fileTags: Record<string, string[]>`) and synchronizes bi-directionally across all member tracks in a song collection.
  * **Real-Time Visual Tag Reflection:** Multi-badge indicators display in the `FILES` tab and dynamically update the overview waveform watermark banner (`getTrackWatermarkLabel`).

### 2.31 Batch Fuzzy Library Auto-Linker & Iso Track Preservation (Milestone 17)
* **Decision:** Automated batch linking across configured rehearsal folders and preservation of isolated accompaniment stems.
* **Implementation Details:**
  * **Dedicated Stems Folder Preference (`prefFolderVocals`):** A 5th designated folder row in Preferences (`VOC`, color `#af52de`) for storing and organizing isolated vocals, lead/backing tracks, and stem outputs.
  * **Batch Fuzzy Linker (`executeAutoLinkLibrary`):** Recursively inspects all five configured asset directories (Performance Tracks, Full-Res Lossless, Original Reference, Sheet Music PDFs, and Isolated Vocals / `Vocals Only/` subdirectories). Using token Jaccard and Levenshtein distance metrics (`calculateMatchScore`), it anchors collections on primary performance tracks and automatically links matching sheet music PDFs ($\ge 0.55$), originals ($\ge 0.60$), lossless masters ($\ge 0.60$), and all corresponding vocal/iso stems ($\ge 0.62$).
  * **Bi-Directional Profile Propagation:** Propagates shared file lists, rehearsal markers, notes, lyrics, and sheet music across all peer tracks, with real-time UI updates for the active track.
  * **Iso Track Instrumental Stem Preservation:** During 4-model ensemble vocal separation (`scripts/uvr_separator.py`), the engine retains the accompaniment output, encoding it to pristine AAC as `[Title] (Iso Track).m4a` alongside `[Title] (Vocals Ensemble).m4a`, automatically tagging it with the new `Iso Track` audio tag (`ISO TRACK` badge `#38bdf8`).
  * **Context-Aware Stem Action Filtering:** Audio tracks identified as Lead Vocals or Backing Vocals automatically suppress the `🎤 Lead/Backups` and `✨ Isolate Vocals` buttons to prevent unnecessary recursive separation passes.

---

## 3. Core Requirements & System Features

### 3.1 Audio Engine & DSP
* Multi-format playback (WAV, AIFF, FLAC, MP3, AAC/M4A, Ogg Vorbis).
* Independent pitch shift and time stretch controls with permanent zero-glitch Signalsmith engagement.
* Parametric EQ with interactive draggable nodes + logarithmic sliders.
* Dual-stage serial/parallel compressor with 4 character models and exact analytical transfer curve.
* Non-destructive gapless cuts and continuous loop wrapping.
* Grand undo/redo, auto-save, and crash recovery.

### 3.2 Waveform & Timeline
* Dual waveform display: Large scrollable/zoomable waveform + mini Overview bar.
* Persistent peak cache.
* Color-coded, draggable, and renamable markers with PDF chart anchor links.
* Multiple concurrent loops with a "Vamp Mode" option (continuous loop until disabled).

### 3.3 Library, Projects & Files
* **Per-Song Persistent Project Profiles:** Preserves all DSP, markers, regions, PDF charts, metadata, and associated tracks.
* **Bi-Directional Song Collections:** Peer-to-peer linking across all audio versions and charts.
* **Live A/B Compare:** Instant seamless flipping between master and stems without playback interruption.
* **Streamlined Single-Line Files Hub:** Space-efficient layout with active-track stem actions.
* **Smooth Type-to-Jump:** Immediate prefix search with smart track number stripping.
* **AI Setlist CSV Importer & Repair Engine:** Easy setlist loading with health verification.
* **Integrated AI Stem Separation:** Headless 4-model ensemble vocal isolation and lead/backing harmony split.

### 3.4 Hardware & External Show Control
* Dedicated Elgato Stream Deck plugin with dynamic LCD feedback.
* WebSocket broadcast server on port `4545`.
* Extensible OSC routing/mapping system with presets for QLab integration.
* Hardware MIDI Learn interface with CC and Note mapping.
* Model Context Protocol (MCP) server for autonomous agent integration.

### 3.5 High-Fidelity Export
* Multi-format offline WAV export (16-bit, 24-bit, 32-bit float) baking DSP, cuts, tempo, pitch, and metadata.

---

## 4. Phase 2 & Later (Future Roadmap)
* **Cross-Platform Windows Release:** Native WASAPI audio thread, MSVC build pipeline, and Windows Recycle Bin integration (Active).
* Elastic Alignment / Sync Anchors (piecewise time-stretch).
* Practice Mode / Rehearsal Sequences (loop chaining with tempo ramps).
* Command Palette (`Cmd+K` / `Ctrl+K`) and custom keyboard shortcuts.
* Full VST/AU plugin hosting implementation (infrastructure stub only in MVP).

---

## 5. Milestone Plan & Progress
* **Milestone 0:** Architecture Decisions & Workspace Skeleton *(Completed)*
* **Milestone 1:** Minimal Viable Playback Engine *(Completed)*
* **Milestone 2:** Pitch and Time Shifting *(Completed)*
* **Milestone 3:** Loops, Markers, and Vamp Mode *(Completed)*
* **Milestone 4:** Persistent Projects & Track Profiles *(Completed)*
* **Milestone 5:** EQ and Compressor DSP *(Completed)*
* **Milestone 6:** Nonlinear Timeline (Cuts) & Splice Crossfades *(Completed)*
* **Milestone 7:** Library Management, Associated Media, and Metadata *(Completed)*
* **Milestone 8:** Stream Deck, WebSocket, MIDI & OSC Show Control *(Completed)*
* **Milestone 9:** High-Fidelity Offline Audio Export Engine *(Completed)*
* **Milestone 10:** AI Stem Separation Engine & macOS Trash Safety *(Completed)*
* **Milestone 11:** Bi-Directional Song Collections, Live A/B Compare & Streamlined Files Hub *(Completed)*
* **Milestone 12:** Setlist CSV Importer, Library Repair Engine & Smooth Type-to-Jump *(Completed)*
* **Milestone 13:** Model Context Protocol (MCP) Server & Automation Tools *(Completed)*
* **Milestone 14:** Cross-Platform Windows Workstation Port *(Completed)*
* **Milestone 15:** Multi-Drive File System & Cloud Navigation *(Completed)*
* **Milestone 16:** Height-Adjustable Waveform & Interactive Audio Tags *(Completed)*
* **Milestone 17:** Batch Fuzzy Library Auto-Linker, Iso Track Preservation, and Stem Workflow Refinements *(Completed)*
