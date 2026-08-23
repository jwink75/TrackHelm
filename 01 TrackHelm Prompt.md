# Master Prompt: “TrackHelm” Professional Music Rehearsal & Playback Workstation

## 0. How to use this document

This is the founding specification for an agentic build. Read it in full before writing any code. Where this document says something is ambiguous or unresolved, do not silently resolve it with a default — flag it and ask, or propose options with trade-offs. Several earlier drafts of this spec quietly narrowed or reinterpreted requirements; treat any implicit interpretation as a risk, not a convenience.

------

## 1. Product Vision for TrackHelm

TrackHelm is not "a DAW" and not "an audio editor." The primary user story is:

> I have a recording, and I want to manipulate it instantly so I can rehearse, learn, analyze, and perform with it.

It's a **professional music rehearsal and playback workstation** with non-destructive audio editing capabilities layered on top of AnyTune Pro+–style rehearsal tools. Every feature decision should be weighed against that story, not against "what would a DAW have." It must be fast and responsive, with no lag between pushing 'play' and the start of playback.

Target: dark-mode native desktop application, optimized for legibility in low-light rehearsal and stage environments.

------

## 2. Development Approach & Rules

These rules govern *how* the build proceeds, independent of feature content:

1. **Architect before implementing.** Milestone 0 (below) must be completed and reported back before any application code is written.
2. **Build incrementally, by milestone** (see §6). Do not jump ahead to later milestones even if it seems efficient.
3. **After each milestone: run tests, verify existing functionality still works, then proceed.** Regressions are not acceptable collateral for new features.
4. **Ambiguity is a stop sign, not a default.** If a requirement in this document is unclear or underspecified, surface it rather than picking an interpretation.
5. **Never modify or overwrite the original source audio file without explicit user confirmation at export time.** All in-app editing is non-destructive by default.
6. **Verify current external APIs rather than relying on training data** for anything involving QLab (OSC/AppleScript interfaces), third-party libraries, or plugin standards — these move over time.
7. **State architectural decisions explicitly**, even when they seem obvious. A one-paragraph rationale in an ADR (architecture decision record) costs little and prevents drift.

------

## 3. Milestone 0: Architecture Decision (required, blocking)

Before any feature code, produce a written architecture decision record covering:

- **Where does the authoritative audio engine live?** Evaluate a native (Rust) real-time audio engine with the UI as a thin client vs. a Web Audio–based engine (e.g., via Tauri's webview). Note explicitly: this project has a stated long-term goal of hosting third-party VST/AU plugins, which a browser sandbox cannot do natively — weigh this constraint directly rather than deciding on convenience alone.
- **Time/pitch engine integration path.** Signalsmith Stretch is a C++ library. Evaluate native FFI/bindings (if a native engine is chosen) vs. WASM compilation (if a web-audio engine is chosen), and the maintenance/latency trade-offs of each.
- **Audio decode library selection** — must support WAV, AIFF, FLAC, MP3, AAC/M4A, and ideally Ogg Vorbis; mono/stereo/multichannel; varying sample rates and bit depths.
- **Persistence architecture** — what lives in structured storage (projects, markers, notes, associations, playlists) vs. what lives in a binary derived-data cache (waveform peaks). Don't assume SQLite is right for both just because it's convenient for one.
- **Plugin hosting feasibility path** — even though VST/AU support is a later milestone, note here what architecture choices now would foreclose or preserve that option later.
- **OSC/MIDI library and command-bus design** — see §4.7; this affects core engine structure, not just a late feature.
- **PDF rendering approach** for the embedded score viewer.

Report the decision and rationale before proceeding to Milestone 1.

------

## 4. Core Requirements (MVP scope)

### 4.1 Audio Engine & DSP

- Multi-format playback: WAV, AIFF, FLAC, MP3, AAC/M4A, ideally Ogg Vorbis. Treat **decode support**, **metadata support**, and **export support** as three separate capability lists — do not assume one implies the others.
- Independent pitch shift and time stretch (Signalsmith Stretch or equivalent quality), controls decoupled from each other.
- Parametric EQ with interactive nodes over a live FFT spectrum display when EQ editing mode is engaged; simple bass/treble shelf controls always available in the main transport.
- Clean feedforward compressor with an on-waveform overlay showing threshold and gain reduction in real time.
- Adjustable per-file playback level.
- **Non-destructive cuts as a nonlinear playback timeline** (not a "playlist scheduler" — the engine must treat `source time A → source time B` as a discontinuity in the playback timeline, correctly interacting with loops, markers, envelopes, waveform display, export, tempo, and pitch). Configurable crossfade duration, no artificial ms range restriction, with sensible defaults and optional zero-crossing snapping.
- Node-based volume envelope: draggable nodes, adjustable curve shape between them, layered over the waveform.
- Live layering: trigger a secondary audio file to play on top of the currently playing one (reference pitches, SFX, click).
- BPM detection; key detection as a stretch goal.
- Reset controls: granular (reset pitch / tempo / EQ / compression / volume / envelope / edits) plus reset-all. Distinguish **Reset** (zero the adjustment) from **Revert** (back to last saved state) from **Undo** (step back through edit history).
- Undo/redo, autosave, and crash recovery are required, not optional, for a professional application.

### 4.2 Waveform & Timeline

- Large scrollable/zoomable main waveform, paired with a small overview/thumbnail display showing full-track position.
- Persistent multi-resolution waveform cache for fast display, with a manual "Rebuild Waveform Cache" command. (Storage mechanism decided in Milestone 0, not dictated here.)
- Draggable, renamable, color-codable markers.
- Multiple concurrent loop regions. Each loop region can independently be set to **Vamp Mode** (plays through continuously until looping is turned off).

### 4.3 Library, Projects & Files

- **Media vs. Project distinction:** *Media* is the source file. A *Project* is the application's interpretation of it — playback settings, markers, loops, skip regions, envelope, EQ, compression, associated files, notes, tags, MIDI/OSC mappings, rehearsal presets. The same media file may have multiple independent projects.
- Folders as smart playlists, sortable multiple ways (filename, date modified, etc.).
- Virtual playlists from multi-selected files (not copied to a new location), reorderable (including custom drag order), with CSV playlist import/export.
- List view: alternating row striping, current file highlights. Toggle display field between filename / title metadata / artist–title / custom.
- Type-ahead search: accumulate characters into a search string with a sliding debounce (~500ms) rather than treating each keystroke as an independent jump; search across title, filename, artist, album, metadata, tags, notes, associated media.
- If a file isn't locally available (e.g., Dropbox), download on attempted playback.
- **Generalized Associated Media system:** an audio item can be linked to an original recording, alternate backing tracks, a performance recording, score PDF, lyrics, MIDI, MusicXML, video, notes, or other files — each relationship typed. Backing track and original recording must be hot-swappable in the main interface without losing either's playback position.
- Embedded PDF viewer linked to the active project for sheet music.
- **Orphan detection via persistent media ID + file fingerprint/hash**, not raw file path — so the app can detect "this file moved/was renamed, here's the likely match" rather than just flagging it missing.
- Metadata: an abstracted **Embedded Metadata** layer with format-specific implementations underneath (don't hard-couple the architecture to ID3). Preserve unknown tags; don't destroy existing metadata on edit; distinguish embedded (in-file) metadata from application-only metadata; support artwork where practical; dedicated lyrics viewer; metadata edits should be undoable where possible. Per-file **application notes** stored independently (not written into the source file), with an orphan-cleanup routine as described above.

### 4.4 Hardware & External Control

- Accept and map MIDI and OSC commands (MIDI Learn style interface).
- Send MIDI/OSC as well as receive.
- Design the application to be **command-oriented internally** — most meaningful operations (play/pause/stop/seek, next/prev track or loop, activate loop, toggle vamp, jump to/set marker, change pitch/tempo/volume, switch associated media, trigger secondary audio) should route through an internal command bus that MIDI/OSC/keyboard/UI can all address. This is an architectural principle to build in from the start, not a v1 feature checklist to fully wire up immediately.
- Emit outgoing status events (playback started/stopped, marker reached, loop started/ended, file changed, playback position) for external listeners.
- macOS automation integration (AppleScript/JXA where appropriate) for controlling QLab — investigate QLab's actual current OSC and AppleScript interfaces rather than assuming a schema. Do not hard-code a specific OSC address scheme (e.g. `/cue/{name}/start`) into the spec; build an extensible OSC routing/mapping system with presets/templates for common QLab operations instead.

### 4.5 Export

- Export the adjusted file including effects and edits (pitch, tempo, EQ, compression, cuts), to common formats (WAV/M4A/MP3 at minimum).
- Warn before overwriting the original file; allow it only with explicit confirmation.
- Highest-quality effects for offline render (same engine quality as real-time where feasible).

### 4.6 Plugin Support (foundation only in MVP)

- Full VST/AU hosting is a later milestone (§7), but Milestone 0's architecture decision must not foreclose it. Begin structuring the audio engine with a plugin-hosting boundary in mind even before plugins are implemented.

### 4.7 UI

- Dark mode throughout, tuned for low-light environments.
- Main interface always shows: transport, simple bass/treble, playback level, waveform + overview, markers/loops.
- Display total time, elapsed time, remaining time in current file.
- Display total time, elapsed time, remaining time in current playlist (or multi-selection).
- Dedicated panels: full metadata viewer, lyrics viewer, associated-media switcher, notes.
- The application should ultimately support a **workspace concept** (e.g., a rehearsal-focused layout vs. an audio/EQ-focused layout vs. a performance/cue-focused layout, with panels shown/hidden or rearranged per workspace) — treat this as a target design direction, not something to fully build before the underlying panels exist.

------

## 5. Phase 2 / Later (explicitly deferred — do not build into MVP)

These are good ideas that should not gate or bloat the initial build. Park them here so they're not lost, and revisit after Milestone 7 or so:

- **Elastic Alignment / Sync Anchors** — piecewise time-stretch alignment between a master track and linked secondary tracks via draggable sync anchor pairs. Build the simpler version first: linked media with independent playback position and an optional fixed offset. Anchors come later — they raise real open questions (behavior outside anchor range, anchor crossing, live anchor dragging during playback, interaction with tempo/loops/export) that deserve their own design pass.
- **Practice Mode / Rehearsal Sequences** — chain loop regions with per-region tempo/pitch/repeat-count settings into an automatic sequence (e.g., mm.12–28 @70% ×4 → mm.29–44 @75% ×4 → …), with count-in (1 beat / 2 beats / 1 bar / 2 bars / custom) and progressive tempo ramps. This is arguably the single most differentiating feature relative to a generic player — sequence its build right after loops/markers (§7, post-Milestone 3) rather than leaving it purely aspirational.
- Command palette (Cmd/Ctrl-K) with searchable commands and assignable keyboard shortcuts for everything.
- A/B processing toggle (instant compare of original vs. current processing state — especially EQ/compression/pitch/tempo).
- Metering: peak, stereo, clipping indicator, output level, compressor gain-reduction meter. RMS/LUFS optional, not central.
- Tags and smart collections for the library.
- Automated audio regression testing as an ongoing practice once there's enough surface area to regress.
- Full VST/AU plugin hosting.

------

## 6. Conceptual Model

```
                         APPLICATION
                              |
              ┌───────────────┴───────────────┐
              |                               |
           LIBRARY                         PROJECT
              |                               |
       ┌──────┴──────┐              ┌─────────┴─────────┐
       |             |              |                   |
     MEDIA       ASSOCIATIONS   PLAYBACK STATE      REHEARSAL
       |             |              |                   |
    Audio          PDF            Pitch              Loops
    Video          MIDI           Tempo              Markers
    Score          Lyrics         EQ                 Sequences
    etc.           Notes          Compression        Presets
                                  Envelope
                                  Cuts
                                  etc.
                              |
                         AUDIO ENGINE
                              |
                     ┌────────┴────────┐
                     |                 |
                 REAL-TIME          OFFLINE
                 PLAYBACK            RENDER
```

------

## 7. Milestone Plan

- **Milestone 0** — Architecture decision document (§3). Blocking.
- **Milestone 1** — Minimal viable playback engine: load audio, waveform display, play/pause, seek, volume, multiple formats.
- **Milestone 2** — Pitch/time shifting.
- **Milestone 3** — Loops and markers, including vamp mode.
- **Milestone 4** — Persistent projects (Media vs. Project model, §4.3).
- **Milestone 5** — EQ and compression.
- **Milestone 6** — Nonlinear playback / cuts (skip regions) and volume envelope.
- **Milestone 7** — Library, metadata, associated files, notes/orphan detection.
- **Milestone 8** — MIDI/OSC/QLab integration (command bus, per §4.4).
- **Milestone 9** — Export.
- **Milestone 10** — Practice Mode / rehearsal sequences.
- **Later** — Elastic alignment (sync anchors), VST/AU plugin hosting, workspaces, command palette, metering, remaining Phase 2 items.

After each milestone: run tests, confirm no regressions, then proceed (Rule 3, §2).



ADDITIONAL INSTRUCTIONS - add to your permanent rules for this project:

- Create a GitHub repository for this project, but as a general rule do not push to GitHub unless explicitly asked.
- Create and maintain a “Project Bible” document in markdown. As with other projects, this should GROW in complexity as the project develops. Never delete or simplify things (things can be CHANGED if we change them during the course of development), only add new features and details. With every major version number update of the bible, create an archival copy in the 
  “zz Project Archive” folder.

