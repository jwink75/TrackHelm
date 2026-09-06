# TrackHelm Windows Implementation Guide 🪟

> **Target Audience:** This document is written specifically for an **Antigravity AI Agent** working on a **Windows PC** to port, build, test, and package **TrackHelm** as a native Windows desktop workstation.

---

## 1. Executive Summary & Windows Target Architecture

TrackHelm is currently developed on macOS (Apple Silicon) as a Tauri v2 application. Because it was architected with modular Rust backends (`trackhelm-engine`), a pure Svelte 4 / HTML5 Canvas frontend, and Symphonia pure-Rust audio decoding, **approximately 90% of the codebase is already cross-platform**.

The remaining 10% consists of platform-specific code that must be adapted for Windows:
1. **C++ FFI Compilation (`signalsmith-stretch-rs`)**: Must compile with MSVC `cl.exe` instead of Clang.
2. **Audio Backend (`cpal`)**: Runs on Windows WASAPI instead of macOS CoreAudio.
3. **Platform Commands (`src-tauri/src/main.rs`)**:
   - Cloud placeholder detection (`is_file_downloaded`): Darwin APFS flags $\to$ Windows Cloud Filter / File Attributes.
   - Safe Trash (`move_file_to_trash`): macOS AppleScript $\to$ Windows Recycle Bin (`trash` crate or Win32 Shell API).
   - File Opener (`open_file_external`): macOS `open` $\to$ Windows `cmd /c start` or `open` crate.
   - Python Virtualenv path: `.venv/bin/python` $\to$ `.venv\Scripts\python.exe`.
   - Remove hardcoded macOS development paths (`/Users/winkler/...`).
4. **Tauri Packaging (`src-tauri/tauri.conf.json`)**: Bundle target `"app"` (macOS bundle) $\to$ `"nsis"` / `"msi"`.
5. **AI Stem Separation (`scripts/uvr_separator.py`)**:
   - AAC encoding: macOS `/usr/bin/afconvert` $\to$ `ffmpeg` via CLI.
   - PyTorch Device: macOS `mps` $\to$ `cuda` (NVIDIA) or `cpu`.
6. **Stream Deck Plugin (`integrations/streamdeck/`)**:
   - Packaging: Bash script $\to$ PowerShell script (`package_plugin.ps1`).
   - Plugin destination: `~/Library/...` $\to$ `%APPDATA%\Elgato\StreamDeck\Plugins`.

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

#### 4.2 PyTorch GPU Acceleration
In `scripts/uvr_separator.py`:
* macOS uses MPS (`torch.backends.mps.is_available()`).
* Windows uses CUDA (`torch.cuda.is_available()`).
* Update device selection logic:
  ```python
  if torch.cuda.is_available():
      device = "cuda"
  elif sys.platform == "darwin" and hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
      device = "mps"
  else:
      device = "cpu"
  ```

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
| :--- | :--- | :--- |
| **1. C++ FFI** | `cargo check -p signalsmith-stretch-rs` | Compiles cleanly with MSVC `cl.exe`. |
| **2. Audio Engine** | `cargo check -p trackhelm-engine` | Compiles with CPAL WASAPI backend. |
| **3. Tauri Core** | `cargo check --manifest-path src-tauri/Cargo.toml` | All IPC commands and crates compile. |
| **4. Frontend** | `npm run build` | Vite builds `dist/` without errors. |
| **5. Dev Mode** | `npm run tauri dev` | TrackHelm desktop window opens with working UI. |
| **6. Audio Playback** | Load audio file (`.wav`, `.mp3`, `.m4a`) | Waveform renders; playback starts instantly via WASAPI. |
| **7. Real-Time DSP** | Drag Speed / Pitch / EQ / Compressor knobs | Pitch/Tempo adjust glitch-free; meters animate. |
| **8. Type-to-Jump** | Type `B` then `E` in playlist/browser | Immediate jump to matching track; indicator displays. |
| **9. Audio Export** | Press `Ctrl+Shift+E` and export WAV | 16/24/32-bit WAV generated with baked DSP. |
| **10. Release Build** | `npm run tauri build` | Generates `.exe` installer in `src-tauri/target/release/bundle/nsis/`. |

---

## 5. Summary of Modified / New Files

When completing the Windows port, the following files will be touched:

1. [`signalsmith-stretch-rs/build.rs`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/signalsmith-stretch-rs/build.rs): Added MSVC flag support (`/std:c++14`, `/EHsc`).
2. [`src-tauri/Cargo.toml`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/src-tauri/Cargo.toml): Added `trash = "5.2"`.
3. [`src-tauri/src/main.rs`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/src-tauri/src/main.rs): Windows cloud attribute detection, Recycle Bin via `trash`, Windows external opener, dynamic Python virtualenv resolution.
4. [`src-tauri/tauri.conf.json`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/src-tauri/tauri.conf.json): Changed bundle targets from `["app"]` to `"all"`.
5. [`scripts/uvr_separator.py`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/scripts/uvr_separator.py): Cross-platform `ffmpeg` AAC encoder and CUDA/CPU device fallback.
6. [`integrations/streamdeck/package_plugin.ps1`](file:///Users/winkler/Library/CloudStorage/Dropbox-Personal/Programming/TrackHelm/integrations/streamdeck/package_plugin.ps1): Windows PowerShell packaging script.
