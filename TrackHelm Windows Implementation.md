# TrackHelm Windows Implementation Guide 🪟

> **Target Audience:** This document is written specifically for an **Antigravity AI Agent** working on a **Windows PC** to port, build, test, and package **TrackHelm** as a native Windows desktop workstation.

---

## 1. Executive Summary & Windows Target Architecture

TrackHelm is fully supported on both **macOS (Apple Silicon & Intel)** and **Windows 10/11 (x64)** as a high-performance Tauri v2 desktop workstation.

All Windows-specific platform tiers have been implemented and verified:
1. **C++ FFI Compilation (`signalsmith-stretch-rs`)**: Native MSVC `cl.exe` compilation (`/std:c++14`, `/EHsc`) with full SIMD vectorization.
2. **Audio Backend & Clock Synchronization (`trackhelm-engine`)**:
   - Real-time low-latency stream running via Windows WASAPI (`cpal`).
   - **Polyphase FFT Resampling (`rubato 0.15`)**: Automatic rate conversion (e.g. 44.1 kHz files $\leftrightarrow$ 48.0 kHz WASAPI DAC hardware clock) with zero latency offset and bit-transparent passthrough.
   - Synchronized DSP (SignalsmithStretch, Biquad EQs, Dual Compressor) calibrated to the hardware's native `device_sample_rate`.
3. **Platform Commands & Multi-Drive File Browser (`src-tauri/src/main.rs`, `src/App.svelte`)**:
   - **Multi-Drive Navigation**: Virtual "This PC" / "Computer" root enumerating all mounted drive letters (`C:`, `D:`, `E:`, `F:`, etc.) with volume labels.
   - **Cloud Sync Bookmarks**: Automatic discovery of active Dropbox, Google Drive, OneDrive, and iCloud storage folders.
   - **Cloud Placeholder Detection**: Windows Cloud Filter API / File Attribute checking (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`).
   - **Safe Recycle Bin Deletion**: Cross-platform file trashing via the `trash` crate.
   - **Windows File Opener**: Native file launching via Windows Shell (`cmd /c start`).
4. **AI Stem Separation (`scripts/uvr_separator.py`, `src-tauri/src/main.rs`)**:
   - **Hardware Acceleration**: NVIDIA CUDA acceleration with CPU fallback.
   - **4-Model Ensemble Dual-Stem Output**: Simultaneously produces isolated master vocals (`(Vocals Ensemble).m4a`) and clean isolated accompaniment (`(Iso Track).m4a`), encoding to pristine AAC via `ffmpeg` on Windows.
   - **Self-Extracting Embedded Fallback**: Worker script embedded into binary via `include_str!` as a failsafe, plus declared in `tauri.conf.json` `bundle.resources`.
   - **Cross-Volume Junction Resolution**: Transparently links pre-downloaded Ultimate Vocal Remover models across NTFS drive junctions without duplicating disk space.
   - **Isolated Python Runtime**: Automatically targets user Python 3.11 with `audio-separator[gpu]` while filtering out conflicting third-party embedded runtimes.
   - **Action Button Filtering**: Sub-stems already classified as Lead or Backing Vocals suppress recursive separation buttons.
5. **Stream Deck Plugin (`integrations/streamdeck/`)**:
   - Packaging and deployment via PowerShell (`package_plugin.ps1`).
6. **Tauri Release Packaging (`src-tauri/tauri.conf.json`)**:
   - Multi-target packaging producing NSIS setup executables (`.exe`) and WiX installer packages (`.msi`).
7. **Batch Fuzzy Library Auto-Linker & Stems Folder**:
   - 5th designated folder row in Preferences: **Isolated Vocals & Stems (UVR)** (`prefFolderVocals`).
   - One-click batch fuzzy linker discovers and pairs performance tracks with matching originals, sheet music PDFs, lossless masters, and all vocal/iso stems across folders and subdirectories.
8. **Milestone 18: Real-Time Audio Engine Safety, PDF Virtualization & Subsystem Hardening**:
   - **Real-Time Thread Safety:** Resampling removed from CPAL callback thread; verified in-flight during decode.
   - **Subprocess Isolation:** AI stem separation runs in `spawn_blocking` to prevent blocking Tokio async workers.
   - **Dynamic PDF Virtualization:** Memory-efficient `IntersectionObserver` with 500px pre-load margin and zero-copy binary IPC (`read_file_binary`).
   - **Decode Deduplication:** Unified `get_or_decode_track` with broadcast channels in `AppState`.
   - **Remote Broadcast Gating:** WebSocket state broadcasting gated by connected clients to eliminate idle serialization overhead.
   - **Drive Cache:** 30s TTL cache on drive and cloud folder discovery.
9. **Milestone 19: Hardware Audio Output Selector, Shift+Wheel Scrubbing & Focus Protection**:
   - **Audio Output Device Selection:** Dedicated host thread architecture managing CPAL stream lifecycle, enabling runtime device hot-switching without COM apartment violations or app restarts. Active audio buffer automatically resampled on-the-fly when device native sample rate differs (e.g. 44.1k to 48k). Preferences modal integration with device persistence to `localStorage`.
   - **Shift + Mousewheel Scrubbing:** Hovering over the main waveform and scrolling with Shift (or horizontal wheel/trackpad) scrubs playhead smoothly with delta dynamically scaled to zoom level (~5% of visible window per notch).
   - **Window Focus Activation Guard:** Prevents activating clicks from other windows from accidentally seeking or rewinding audio by ignoring waveform clicks within 250ms of window focus. Bare Enter key decoupled from stop/rewind. Transport buttons automatically blurred on click.

---

## 2. Windows Prerequisites & Environment Setup Checklist

Before running any builds, ensure the Windows PC has the following tools installed and configured in system `%PATH%`:

### 2.1 C++ Build Tools & MSVC
* Install **Visual Studio 2022 Community** (or Visual Studio Build Tools).
* In the Visual Studio Installer, check **Desktop development with C++**:
  * MSVC v143 - VS 2022 C++ x64/x86 build tools.
  * Windows 10 SDK (or Windows 11 SDK `10.0.22621.0`+).
  * C++ CMake tools for Windows.
* Verify MSVC from PowerShell:
  ```powershell
  cl.exe
  # Should output: Microsoft (R) C/C++ Optimizing Compiler ... for x64
  ```

### 2.2 Rust Toolchain
* Install Rust via [rustup.rs](https://rustup.rs/):
  ```powershell
  rustup default stable-x86_64-pc-windows-msvc
  rustup update
  ```
* Verify:
  ```powershell
  rustc --version
  cargo --version
  ```

### 2.3 Node.js & Package Manager
* Install **Node.js LTS (v18 or v20)** from [nodejs.org](https://nodejs.org/).
* Verify:
  ```powershell
  node --version
  npm --version
  ```

### 2.4 Python (For UVR Stem Separation)
* Install **Python 3.10 or 3.11** (Ensure "Add Python to PATH" is checked).
* Verify:
  ```powershell
  python --version
  ```

### 2.5 FFmpeg (For AAC Encoding on Windows)
* Download FFmpeg (gyan.dev or via `winget install Gyan.FFmpeg`).
* Verify:
  ```powershell
  ffmpeg -version
  ```

### 2.6 Windows WebView2 Runtime
* Windows 10/11 already includes the **Microsoft Edge WebView2 Runtime** by default.
* If missing, install via `winget install Microsoft.EdgeWebView2Runtime`.

---

## 3. Step-by-Step Implementation Roadmap

### Step 1: Adapt C++ FFI Build (`signalsmith-stretch-rs/build.rs`)

`signalsmith-stretch-rs` compiles `signalsmith-stretch` via the `cc` crate. On Windows with MSVC, we need to ensure the MSVC compiler flags are set properly.

**Inspect `signalsmith-stretch-rs/build.rs`:**
```rust
fn main() {
    let mut build = cc::Build::new();
    build.cpp(true);
    build.file("src/wrapper.cpp");
    build.include("."); // include crate root for headers

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        build.flag("/std:c++14"); // Signalsmith requires C++11 or higher; MSVC supports /std:c++14 or c++17
        build.flag("/EHsc");      // Enable C++ standard exception handling
        build.flag("/O2");        // Enable speed optimizations
    } else {
        build.std("c++11");
    }

    build.compile("signalsmith_stretch");

    println!("cargo:rerun-if-changed=src/wrapper.cpp");
    println!("cargo:rerun-if-changed=signalsmith-stretch.h");
}
```

**Verification:**
```powershell
cargo check -p signalsmith-stretch-rs
cargo test -p signalsmith-stretch-rs
```

---

### Step 2: Adapt Audio Backend in `trackhelm-engine`

`trackhelm-engine` uses `cpal` for real-time audio playback.
* On macOS, `cpal` uses the `CoreAudio` backend.
* On Windows, `cpal` automatically uses the `WASAPI` backend.

**Key WASAPI considerations:**
1. **Buffer Size & Latency:**
   - In `trackhelm-engine/src/engine.rs`, inspect how `StreamConfig` is initialized:
   ```rust
   let device = host.default_output_device().ok_or("No default output device")?;
   let config: cpal::StreamConfig = device.default_output_config()?.into();
   ```
   - On Windows WASAPI, `config.buffer_size` may be `BufferSize::Default`. If low-latency glitches occur, request an explicit buffer frame size (e.g. `BufferSize::Fixed(512)` or `1024`).
2. **Sample Format Conversion:**
   - WASAPI devices often default to `f32` or `i16` at $44.1\text{ kHz}$ or $48\text{ kHz}$. Ensure `build_output_stream` in `trackhelm-engine` handles `SampleFormat::F32` and `SampleFormat::I16` cleanly.

**Verification:**
```powershell
cargo check -p trackhelm-engine
```

---

### Step 3: Refactor Platform-Specific Tauri Commands (`src-tauri/src/main.rs`)

#### 3.1 Cloud Placeholder Detection (`is_file_downloaded`)
On macOS, APFS uses `UF_DATALESS` (`0x40000000`) and `blocks == 0`.
On Windows (OneDrive / Dropbox / iCloud for Windows):
* Files not stored locally have the Win32 attribute `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (`0x00400000`) or `FILE_ATTRIBUTE_OFFLINE` (`0x00001000`).
* Replace `is_file_downloaded` with a cross-platform implementation:

```rust
#[tauri::command]
fn is_file_downloaded(path: String) -> bool {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt as DarwinMetadataExt;
        use std::os::unix::fs::MetadataExt as UnixMetadataExt;
        if let Ok(meta) = p.metadata() {
            let flags = DarwinMetadataExt::st_flags(&meta);
            const UF_DATALESS: u32 = 0x4000_0000;
            if (flags & UF_DATALESS) != 0 {
                return false;
            }
            if meta.len() > 0 && UnixMetadataExt::blocks(&meta) == 0 {
                return false;
            }
            return true;
        }
        false
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = p.metadata() {
            let attrs = meta.file_attributes();
            const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
            const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
            if (attrs & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_OFFLINE)) != 0 {
                return false;
            }
            return true;
        }
        false
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        p.exists()
    }
}
```

#### 3.2 Safe File Deletion to Recycle Bin (`move_file_to_trash`)
Instead of calling `osascript` on macOS and `remove_file` on Windows:
Add the cross-platform [`trash`](https://crates.io/crates/trash) crate to `src-tauri/Cargo.toml`:
```toml
[dependencies]
trash = "5.2"
```
Then simplify `move_file_to_trash`:
```rust
#[tauri::command]
fn move_file_to_trash(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("File does not exist: {}", path));
    }
    trash::delete(p).map_err(|e| format!("Failed to move file to Recycle Bin: {}", e))
}
```
*Benefits:* Works natively on macOS Trash (`~/.Trash`), Windows Recycle Bin (`SHFileOperation`), and Linux FreeDesktop trash!

#### 3.3 Open External File (`open_file_external`)
On Windows, `open` does not exist:
```rust
#[tauri::command]
fn open_file_external(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(&["/C", "start", "", &path])
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}
```

#### 3.4 Python & Virtual Environment Discovery in `run_uvr_separation`
On Windows, Python virtualenv binaries are located in `.venv\Scripts\python.exe` (not `.venv/bin/python`).
Also remove any hardcoded `/Users/winkler/...` fallback paths:

```rust
// Look for virtualenv python
let mut found_py = None;
for root in &candidate_roots {
    #[cfg(target_os = "windows")]
    {
        let venv_py = root.join(".venv").join("Scripts").join("python.exe");
        if venv_py.exists() {
            found_py = Some(venv_py);
            break;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let venv_py = root.join(".venv/bin/python");
        if venv_py.exists() {
            found_py = Some(venv_py);
            break;
        }
        let venv_py3 = root.join(".venv/bin/python3");
        if venv_py3.exists() {
            found_py = Some(venv_py3);
            break;
        }
    }
}

#[cfg(target_os = "windows")]
let py = found_py.unwrap_or_else(|| std::path::PathBuf::from("python.exe"));

#[cfg(not(target_os = "windows"))]
let py = found_py.unwrap_or_else(|| std::path::PathBuf::from("python3"));
```

---

### Step 4: Adapt UVR Worker Script (`scripts/uvr_separator.py`)

#### 4.1 Replace `afconvert` with `ffmpeg`
In `scripts/uvr_separator.py`:
```python
def convert_wav_to_aac(wav_path: str, m4a_path: str, bitrate: int = 256000) -> bool:
    """Encodes WAV to AAC .m4a using native afconvert (macOS) or ffmpeg (Windows/Linux)."""
    if sys.platform == "darwin" and os.path.exists("/usr/bin/afconvert"):
        cmd = [
            "/usr/bin/afconvert",
            "-f", "m4af",
            "-d", "aac",
            "-b", str(bitrate),
            str(wav_path),
            str(m4a_path)
        ]
    else:
        # Cross-platform fallback via ffmpeg
        bitrate_k = f"{int(bitrate / 1000)}k"
        cmd = [
            "ffmpeg",
            "-y",
            "-i", str(wav_path),
            "-c:a", "aac",
            "-b:a", bitrate_k,
            str(m4a_path)
        ]

    try:
        res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        return res.returncode == 0
    except Exception as e:
        emit_event("log", message=f"Audio encoding failed: {e}")
        return False
```

#### 4.2 PyTorch & ONNX Runtime GPU Acceleration (CUDA 12.4)
In `scripts/uvr_separator.py`:
* macOS uses MPS (`torch.backends.mps.is_available()`).
* Windows uses CUDA (`torch.cuda.is_available()`).
* On Windows, Python 3.8+ does not automatically inherit DLL search paths from `%PATH%`. To enable CUDA for both PyTorch (Demucs, MDX23C, 5HP Karaoke) and ONNX Runtime (Kim Vocal 2, UVR-MDX-NET):
  ```python
  if sys.platform == "win32":
      try:
          import torch
          torch_lib = os.path.join(os.path.dirname(torch.__file__), "lib")
          if os.path.isdir(torch_lib):
              os.add_dll_directory(torch_lib)
              os.environ["PATH"] = torch_lib + os.pathsep + os.environ.get("PATH", "")
      except Exception:
          pass
  ```
* Combined with `torch 2.6.0+cu124` and `onnxruntime-gpu 1.19.2`, hardware acceleration executes on NVIDIA GPUs (e.g. GeForce RTX 2070), reducing ensemble runtime from ~55 minutes on CPU down to ~3–4 minutes.

#### 4.3 Model Discovery & Automated Download Fallback
* `ensure_model_symlinks` checks `%LOCALAPPDATA%\Programs\Ultimate Vocal Remover\models` to link existing model weights directly into `%LOCALAPPDATA%\TrackHelm\models_cache\`.
* If UVR is not present or specific models are missing, `audio-separator` automatically downloads the necessary model weights on demand from HuggingFace/GitHub into the cache.

#### 4.4 Clean Stderr / Stdout Event Stream Separation
* Python informational status records (`INFO`, `WARNING`, `tqdm` iterations) are routed through `sys.stdout` or filtered in the Rust host (`main.rs`).
* In `src-tauri/src/main.rs`, `run_uvr_separation` parses JSON event messages from `stdout` and filters `stderr` to isolate true Python Tracebacks or non-zero exit codes, preventing benign startup logs from appearing as red error banners in the UI.

#### 4.5 Windowless Background Execution (`CREATE_NO_WINDOW`)
* On Windows, GUI applications spawning console executables (such as `python.exe` or `ffmpeg.exe`) will trigger Windows to display a black command prompt/terminal window by default.
* In `src-tauri/src/main.rs`, added Windows process creation flags via `std::os::windows::process::CommandExt`:
  ```rust
  #[cfg(target_os = "windows")]
  {
      use std::os::windows::process::CommandExt;
      const CREATE_NO_WINDOW: u32 = 0x08000000;
      cmd.creation_flags(CREATE_NO_WINDOW);
  }
  ```
* In `scripts/uvr_separator.py`, added `creationflags=subprocess.CREATE_NO_WINDOW` to `subprocess.run` calls (e.g. `ffmpeg` AAC conversion), guaranteeing that vocal isolation runs completely silently in the background with zero visible terminal windows.

---

### Step 5: Update `tauri.conf.json` for Windows Bundling

In `src-tauri/tauri.conf.json`:
Currently:
```json
"bundle": {
  "active": true,
  "targets": ["app"]
}
```
Update to:
```json
"bundle": {
  "active": true,
  "targets": "all",
  "icon": [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/128x128@2x.png",
    "icons/icon.icns",
    "icons/icon.ico"
  ],
  "windows": {
    "nsis": {
      "installMode": "currentUser"
    }
  }
}
```
* `icon.ico` is already present in `src-tauri/icons/icon.ico`.
* `"targets": "all"` will build `.app` on macOS and NSIS / MSI installers on Windows.

---

### Step 6: Create Stream Deck Packaging Script for Windows (`package_plugin.ps1`)

Create `integrations/streamdeck/package_plugin.ps1`:
```powershell
$ErrorActionPreference = "Stop"

$Dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$OutDir = Join-Path $Dir "..\..\dist-streamdeck"

if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Path $OutDir | Out-Null
}

Write-Host "1. Generating action icons..."
python "$Dir\generate_icons.py"

Write-Host "2. Packaging .streamDeckPlugin archive..."
$PluginDir = Join-Path $Dir "com.trackhelm.controller.sdPlugin"
$ZipOut = Join-Path $OutDir "com.trackhelm.controller.streamDeckPlugin"
$AliasOut = Join-Path $OutDir "TrackHelm.streamDeckPlugin"

if (Test-Path $ZipOut) { Remove-Item $ZipOut -Force }
if (Test-Path $AliasOut) { Remove-Item $AliasOut -Force }

Compress-Archive -Path "$PluginDir\*" -DestinationPath $ZipOut
Copy-Item -Path $ZipOut -Destination $AliasOut

Write-Host "✓ Created: dist-streamdeck\TrackHelm.streamDeckPlugin"

# 3. Direct install to Stream Deck Plugins directory on Windows
$AppData = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::ApplicationData)
$StreamDeckPlugins = Join-Path $AppData "Elgato\StreamDeck\Plugins"

if (Test-Path $StreamDeckPlugins) {
    $TargetDir = Join-Path $StreamDeckPlugins "com.trackhelm.controller.sdPlugin"
    if (Test-Path $TargetDir) { Remove-Item $TargetDir -Recurse -Force }
    Copy-Item -Path $PluginDir -Destination $TargetDir -Recurse
    Write-Host "✓ Installed directly to: $TargetDir"
}
```

---

### Step 7: Frontend Keyboard & UI Verification

In `src/App.svelte`:
- TrackHelm already checks `(e.metaKey || e.ctrlKey)` for shortcuts (`Cmd+,` / `Ctrl+,`, `Cmd+Shift+E` / `Ctrl+Shift+E`, etc.).
- The Typeahead engine captures alphanumeric characters and uses `behavior: "smooth"` scrolling, which is fully supported by WebView2.
- SVG Canvas and WebGL / 2D Canvas rendering use standard HTML5 APIs identical across Chrome/Edge/Safari.

---

## 4. Verification & Testing Playbook for the Windows Agent

Follow this checklist to verify each tier on the Windows PC:

| Step | Command | Expected Result |
| Step | Command | Expected Result | Status |
| :--- | :--- | :--- | :--- |
| **1. C++ FFI** | `cargo check -p signalsmith-stretch-rs` | Compiles cleanly with MSVC `cl.exe`. | **Verified ✓** |
| **2. Audio Engine** | `cargo check -p trackhelm-engine` | Compiles with CPAL WASAPI backend. | **Verified ✓** |
| **3. Resampler Unit Tests** | `cargo test -p trackhelm-engine` | 44.1k $\leftrightarrow$ 48k polyphase tests pass. | **Verified ✓** |
| **4. Tauri Core** | `cargo check --manifest-path src-tauri/Cargo.toml` | All IPC commands and crates compile. | **Verified ✓** |
| **5. Frontend** | `npm run build` | Vite builds `dist/` without errors. | **Verified ✓** |
| **6. Dev Mode** | `npm run tauri dev` | TrackHelm desktop window opens with working UI. | **Verified ✓** |
| **7. Audio Playback** | Load audio file (`.wav`, `.mp3`, `.m4a`) | Waveform renders; playback starts instantly via WASAPI. | **Verified ✓** |
| **8. Real-Time DSP** | Drag Speed / Pitch / EQ / Compressor knobs | Pitch/Tempo adjust glitch-free; meters animate. | **Verified ✓** |
| **9. Resampling Fidelity** | Play 44.1k file on 48k device | True A440 pitch, 1.000× duration, zero latency offset. | **Verified ✓** |
| **10. Multi-Drive Browser** | Navigate to "This PC" root | Browse C:, D:, E:, and cloud sync folders. | **Verified ✓** |
| **11. Vocal Isolation (UVR)** | Click `✨ Isolate Vocals` | CUDA/CPU separation runs via embedded worker. | **Verified ✓** |
| **12. Type-to-Jump** | Type `B` then `E` in playlist/browser | Immediate jump to matching track; indicator displays. | **Verified ✓** |
| **13. Audio Export** | Press `Ctrl+Shift+E` and export WAV | 16/24/32-bit WAV generated with baked DSP. | **Verified ✓** |
| **14. Release Build** | `npm run tauri build` | Generates NSIS `.exe` and WiX `.msi` installers. | **Verified ✓** |

---

## 5. Summary of Modified / New Files

1. [`signalsmith-stretch-rs/build.rs`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/signalsmith-stretch-rs/build.rs): Added MSVC compiler support (`/std:c++14`, `/EHsc`).
2. [`trackhelm-engine/Cargo.toml`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/trackhelm-engine/Cargo.toml): Integrated `rubato = "0.15"`.
3. [`trackhelm-engine/src/resampler.rs`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/trackhelm-engine/src/resampler.rs): High-fidelity polyphase FFT resampler with delay compensation, filter tail flushing, and automated unit tests.
4. [`trackhelm-engine/src/engine.rs`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/trackhelm-engine/src/engine.rs): Audio engine device clock synchronization, bit-transparent bypass, and calibrated DSP time constants.
5. [`src-tauri/Cargo.toml`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/src-tauri/Cargo.toml): Added `trash = "5.2"`.
6. [`src-tauri/src/main.rs`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/src-tauri/src/main.rs):
   - Multi-tier script discovery with self-extracting embedded fallback (`include_str!`).
   - Isolated Windows Python 3.11 discovery.
   - Background audio pre-resampling and peak generation.
   - Windows file openers, cloud file attribute checks, and Recycle Bin integration.
   - Asynchronous subprocess stderr capture.
7. [`src-tauri/tauri.conf.json`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/src-tauri/tauri.conf.json): Multi-target bundling (`"targets": "all"`), NSIS currentUser configuration, and script resource bundling (`"resources": ["../scripts/*"]`).
8. [`scripts/uvr_separator.py`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/scripts/uvr_separator.py): Cross-platform `ffmpeg` AAC encoding, CUDA/MPS device selection, `--cache-dir` support, and cross-volume NTFS junction model linking.
9. [`src/App.svelte`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/src/App.svelte): Windows "This PC" virtual root browsing, drive letters, cloud bookmark shortcuts, and cross-platform keyboard shortcuts (`Ctrl+` / `Cmd+`).
10. [`integrations/streamdeck/package_plugin.ps1`](file:///F:/Dropbox%20%28Personal%29/Programming/TrackHelm/integrations/streamdeck/package_plugin.ps1): Windows PowerShell packaging script.
