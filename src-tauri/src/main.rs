#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{State, Emitter, Manager};
use tauri::menu::{Menu, MenuItem, Submenu, PredefinedMenuItem};
use trackhelm_engine::{Command, CommandBus, SharedEngineState, DecodedAudio, decode_file, resample_audio_channels};
use lofty::prelude::*;

mod control;

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Clone, serde::Serialize)]
struct TrackMetadata {
    duration_seconds: f64,
    sample_rate: u32,
    channels: usize,
    overview_peaks: Vec<f32>,
    pyramid_peaks: Vec<f32>,
}

struct CachedTrack {
    audio: Arc<DecodedAudio>,
    metadata: TrackMetadata,
    modified_time: std::time::SystemTime,
    file_size: u64,
}

struct LruTrackCache {
    map: HashMap<String, Arc<CachedTrack>>,
    order: Vec<String>,
    max_items: usize,
}

impl LruTrackCache {
    fn new(max_items: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: Vec::new(),
            max_items: max_items.max(2),
        }
    }

    fn get(&mut self, path: &str) -> Option<Arc<CachedTrack>> {
        if let Some(track) = self.map.get(path).cloned() {
            // Touch LRU order (move to most recent position)
            if let Some(pos) = self.order.iter().position(|p| p == path) {
                self.order.remove(pos);
            }
            self.order.push(path.to_string());
            Some(track)
        } else {
            None
        }
    }

    fn insert(&mut self, path: String, track: Arc<CachedTrack>) {
        if let Some(pos) = self.order.iter().position(|p| p == &path) {
            self.order.remove(pos);
        }
        self.order.push(path.clone());
        self.map.insert(path, track);

        // Evict oldest decoded tracks if over maximum bounded limit
        while self.map.len() > self.max_items && !self.order.is_empty() {
            let oldest_path = self.order.remove(0);
            self.map.remove(&oldest_path);
            eprintln!("LRU Cache evicted decoded track: {}", oldest_path);
        }
    }
}

fn get_file_mtime_and_size(path: &str) -> (std::time::SystemTime, u64) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let size = meta.len();
        (mtime, size)
    } else {
        (std::time::SystemTime::UNIX_EPOCH, 0)
    }
}

struct AppState {
    command_bus: CommandBus,
    shared_engine_state: Arc<SharedEngineState>,
    active_audio: Mutex<Option<Arc<DecodedAudio>>>,
    track_cache: Mutex<LruTrackCache>,
    ws_state: Arc<control::websocket::WebSocketServerState>,
    midi_manager: Arc<control::midi::MidiManager>,
}

#[derive(serde::Serialize)]
struct PlaybackStatus {
    is_playing: bool,
    current_time: f64,
    duration_seconds: f64,
    progress: f32,
    in_peak_l: f32,
    in_peak_r: f32,
    out_peak_l: f32,
    out_peak_r: f32,
    gr_stage1: f32,
    gr_stage2: f32,
}

#[derive(serde::Serialize, Clone)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
    size_bytes: u64,
    kind: String, // "drive" | "cloud" | "dir" | "file"
}

#[derive(serde::Serialize)]
struct DirContents {
    current_path: String,
    parent_path: Option<String>,
    entries: Vec<DirEntry>,
    drives: Vec<DirEntry>,
    cloud_folders: Vec<DirEntry>,
    root_name: String,
    home_path: String,
}

fn get_computer_root_name() -> &'static str {
    #[cfg(target_os = "windows")]
    { "This PC" }
    #[cfg(target_os = "macos")]
    { "This Mac" }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    { "This Computer" }
}

#[cfg(target_os = "windows")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
    fn GetVolumeInformationW(
        lpRootPathName: *const u16,
        lpVolumeNameBuffer: *mut u16,
        nVolumeNameSize: u32,
        lpVolumeSerialNumber: *mut u32,
        lpMaximumComponentLength: *mut u32,
        lpFileSystemFlags: *mut u32,
        lpFileSystemNameBuffer: *mut u16,
        nFileSystemNameSize: u32,
    ) -> i32;
}

fn get_system_drives() -> Vec<DirEntry> {
    #[cfg(target_os = "windows")]
    {
        let mut drives = Vec::new();
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26 {
            if (mask & (1 << i)) != 0 {
                let letter = (b'A' + i as u8) as char;
                let root_str = format!("{}:\\", letter);
                let wide_path: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
                
                let drive_type = unsafe { GetDriveTypeW(wide_path.as_ptr()) };
                // 1 = DRIVE_NO_ROOT_DIR (skip unmounted drive letters)
                if drive_type <= 1 {
                    continue;
                }

                let mut vol_name = [0u16; 260];
                let success = unsafe {
                    GetVolumeInformationW(
                        wide_path.as_ptr(),
                        vol_name.as_mut_ptr(),
                        vol_name.len() as u32,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                    )
                };

                // Skip optical drives that have no disc media inside
                if drive_type == 5 && success == 0 {
                    continue;
                }

                let label = if success != 0 {
                    let len = vol_name.iter().position(|&c| c == 0).unwrap_or(vol_name.len());
                    String::from_utf16_lossy(&vol_name[..len]).trim().to_string()
                } else {
                    String::new()
                };

                let display_name = if !label.is_empty() {
                    format!("{} ({}:)", label, letter)
                } else {
                    match drive_type {
                        2 => format!("Removable Disk ({}:)", letter),
                        3 => format!("Local Disk ({}:)", letter),
                        4 => format!("Network Drive ({}:)", letter),
                        5 => format!("CD Drive ({}:)", letter),
                        _ => format!("Drive ({}:)", letter),
                    }
                };

                drives.push(DirEntry {
                    name: display_name,
                    path: root_str,
                    is_dir: true,
                    size_bytes: 0,
                    kind: "drive".to_string(),
                });
            }
        }
        drives
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut roots = Vec::new();
        #[cfg(target_os = "macos")]
        let root_label = "Macintosh HD (/)";
        #[cfg(not(target_os = "macos"))]
        let root_label = "Root (/)";

        roots.push(DirEntry {
            name: root_label.to_string(),
            path: "/".to_string(),
            is_dir: true,
            size_bytes: 0,
            kind: "drive".to_string(),
        });

        let volumes = std::path::Path::new("/Volumes");
        if volumes.exists() && volumes.is_dir() {
            if let Ok(entries) = std::fs::read_dir(volumes) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let p = entry.path();
                    if let Ok(canon) = p.canonicalize() {
                        if canon == std::path::Path::new("/") {
                            continue;
                        }
                    }
                    roots.push(DirEntry {
                        name: format!("{} (Volume)", name),
                        path: p.to_string_lossy().to_string(),
                        is_dir: true,
                        size_bytes: 0,
                        kind: "drive".to_string(),
                    });
                }
            }
        }
        roots
    }
}

fn detect_cloud_folders(drives: &[DirEntry]) -> Vec<DirEntry> {
    let mut cloud_folders: Vec<DirEntry> = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    let mut add_cloud_folder = |name: String, path_buf: std::path::PathBuf| {
        if !path_buf.exists() || !path_buf.is_dir() {
            return;
        }
        let clean_path = if let Ok(canon) = path_buf.canonicalize() {
            let s = canon.to_string_lossy().to_string();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        } else {
            let s = path_buf.to_string_lossy().to_string();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        };

        let key = clean_path.to_lowercase();
        if !seen_paths.contains(&key) {
            seen_paths.insert(key);
            cloud_folders.push(DirEntry {
                name,
                path: clean_path,
                is_dir: true,
                size_bytes: 0,
                kind: "cloud".to_string(),
            });
        }
    };

    let home = dirs::home_dir();

    // 1. Dropbox info.json (Windows %LOCALAPPDATA% / %APPDATA%, macOS ~/.dropbox)
    #[cfg(target_os = "windows")]
    {
        let mut dropbox_json_paths = Vec::new();
        if let Ok(lad) = std::env::var("LOCALAPPDATA") {
            dropbox_json_paths.push(std::path::PathBuf::from(lad).join("Dropbox").join("info.json"));
        }
        if let Ok(ad) = std::env::var("APPDATA") {
            dropbox_json_paths.push(std::path::PathBuf::from(ad).join("Dropbox").join("info.json"));
        }

        for j_path in dropbox_json_paths {
            if j_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&j_path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(obj) = json.as_object() {
                            for (key, val) in obj {
                                if let Some(p) = val.get("path").and_then(|v| v.as_str()) {
                                    let pb = std::path::PathBuf::from(p);
                                    if pb.exists() {
                                        let name = if key == "personal" {
                                            "Dropbox (Personal)".to_string()
                                        } else if key == "business" {
                                            "Dropbox (Business)".to_string()
                                        } else {
                                            format!("Dropbox ({})", key)
                                        };
                                        add_cloud_folder(name, pb);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(ref h) = home {
            let j_path = h.join(".dropbox").join("info.json");
            if j_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&j_path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(obj) = json.as_object() {
                            for (key, val) in obj {
                                if let Some(p) = val.get("path").and_then(|v| v.as_str()) {
                                    let pb = std::path::PathBuf::from(p);
                                    if pb.exists() {
                                        let name = if key == "personal" {
                                            "Dropbox (Personal)".to_string()
                                        } else if key == "business" {
                                            "Dropbox (Business)".to_string()
                                        } else {
                                            format!("Dropbox ({})", key)
                                        };
                                        add_cloud_folder(name, pb);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. macOS ~/Library/CloudStorage (Monterey+ FileProvider) & iCloud Drive
    #[cfg(target_os = "macos")]
    {
        if let Some(ref h) = home {
            let cs = h.join("Library/CloudStorage");
            if cs.exists() && cs.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&cs) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let raw_name = entry.file_name().to_string_lossy().to_string();
                            if !raw_name.starts_with('.') {
                                let display_name = if raw_name.starts_with("Dropbox") {
                                    raw_name.replace('-', " ")
                                } else if raw_name.starts_with("GoogleDrive") {
                                    format!("Google Drive ({})", raw_name.trim_start_matches("GoogleDrive-"))
                                } else if raw_name.starts_with("OneDrive") {
                                    format!("OneDrive ({})", raw_name.trim_start_matches("OneDrive-"))
                                } else if raw_name.starts_with("Box") {
                                    "Box".to_string()
                                } else {
                                    raw_name
                                };
                                add_cloud_folder(display_name, path);
                            }
                        }
                    }
                }
            }

            let icloud = h.join("Library/Mobile Documents/com~apple~CloudDocs");
            if icloud.exists() && icloud.is_dir() {
                add_cloud_folder("iCloud Drive".to_string(), icloud);
            }
        }
    }

    // 3. OneDrive environment variables (Windows)
    #[cfg(target_os = "windows")]
    {
        for env_var in &["OneDrive", "OneDriveCommercial", "OneDriveConsumer"] {
            if let Ok(val) = std::env::var(env_var) {
                if !val.is_empty() {
                    let p = std::path::PathBuf::from(&val);
                    if p.exists() {
                        let name = if *env_var == "OneDriveCommercial" {
                            "OneDrive (Business)".to_string()
                        } else {
                            "OneDrive".to_string()
                        };
                        add_cloud_folder(name, p);
                    }
                }
            }
        }
    }

    // 4. Common user home cloud folders
    if let Some(ref h) = home {
        let common = [
            ("Dropbox", "Dropbox"),
            ("OneDrive", "OneDrive"),
            ("Google Drive", "Google Drive"),
            ("iCloudDrive", "iCloud Drive"),
            ("Box", "Box"),
        ];
        for (sub, name) in common {
            let p = h.join(sub);
            if p.exists() && p.is_dir() {
                add_cloud_folder(name.to_string(), p);
            }
        }
    }

    // 5. Scan drive roots for common cloud folders (e.g. F:\Dropbox (Personal))
    for drive in drives {
        let root = std::path::Path::new(&drive.path);
        let cloud_candidates = [
            "Dropbox",
            "Dropbox (Personal)",
            "Dropbox (Business)",
            "Google Drive",
            "OneDrive",
            "My Drive",
        ];
        for cand in cloud_candidates {
            let cand_path = root.join(cand);
            if cand_path.exists() && cand_path.is_dir() {
                add_cloud_folder(cand.to_string(), cand_path);
            }
        }
    }

    cloud_folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    cloud_folders
}

#[tauri::command]
fn get_cloud_folders() -> Result<Vec<DirEntry>, String> {
    let drives = get_system_drives();
    Ok(detect_cloud_folders(&drives))
}

#[tauri::command]
fn read_dir(path: Option<String>) -> Result<DirContents, String> {
    use std::path::PathBuf;

    let root_name = get_computer_root_name().to_string();
    let all_drives = get_system_drives();
    let all_cloud = detect_cloud_folders(&all_drives);

    let home = dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())?;
    let home_str = home.to_string_lossy().to_string();
    let clean_home = if let Some(stripped) = home_str.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        home_str
    };

    // Check if client requested "This PC" / "This Mac" / computer overview
    if let Some(ref p) = path {
        let p_trimmed = p.trim();
        if p_trimmed == root_name 
            || p_trimmed == "This PC" 
            || p_trimmed == "This Mac" 
            || p_trimmed == "drives" 
            || p_trimmed == "computer" 
        {
            let mut computer_entries = Vec::new();
            for d in &all_drives {
                computer_entries.push(d.clone());
            }
            for c in &all_cloud {
                computer_entries.push(c.clone());
            }

            return Ok(DirContents {
                current_path: root_name.clone(),
                parent_path: None,
                entries: computer_entries,
                drives: all_drives,
                cloud_folders: all_cloud,
                root_name,
                home_path: clean_home,
            });
        }
    }

    let target_path = match path {
        Some(p) => {
            #[allow(unused_mut)]
            let mut p_str = p.trim().to_string();
            // On Windows, if user passes "F:" without backslash, add backslash to refer to root
            #[cfg(target_os = "windows")]
            {
                if p_str.len() == 2 && p_str.ends_with(':') {
                    p_str.push('\\');
                }
            }

            if p_str.starts_with('~') {
                if p_str.len() > 2 {
                    home.join(&p_str[2..]) // Skip "~/"
                } else {
                    home.clone()
                }
            } else {
                PathBuf::from(p_str)
            }
        }
        None => home.clone(),
    };

    let canonical = target_path.canonicalize()
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    let canonical_str = canonical.to_string_lossy().to_string();
    let clean_current = if let Some(stripped) = canonical_str.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        canonical_str
    };

    // Determine parent path: at drive roots (C:\, F:\, /), going up goes to "This PC" / "This Mac"
    let parent_path = match canonical.parent() {
        Some(p) => {
            let ps = p.to_string_lossy().to_string();
            let clean_p = if let Some(stripped) = ps.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                ps
            };
            if clean_p.is_empty() || clean_p.ends_with(':') {
                Some(root_name.clone())
            } else {
                Some(clean_p)
            }
        }
        None => Some(root_name.clone()),
    };

    let mut entries = Vec::new();
    let read_entries = std::fs::read_dir(&canonical)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_entries {
        if let Ok(entry) = entry {
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);

            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') {
                continue;
            }

            let path_raw = entry.path().to_string_lossy().to_string();
            let clean_entry_path = if let Some(stripped) = path_raw.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                path_raw
            };
            let size_bytes = metadata.map(|m| m.len()).unwrap_or(0);

            if is_dir {
                entries.push(DirEntry {
                    name,
                    path: clean_entry_path,
                    is_dir: true,
                    size_bytes,
                    kind: "dir".to_string(),
                });
            } else {
                let lower_name = name.to_lowercase();
                if lower_name.ends_with(".wav")
                    || lower_name.ends_with(".mp3")
                    || lower_name.ends_with(".flac")
                    || lower_name.ends_with(".m4a")
                    || lower_name.ends_with(".aiff")
                    || lower_name.ends_with(".ogg")
                {
                    entries.push(DirEntry {
                        name,
                        path: clean_entry_path,
                        is_dir: false,
                        size_bytes,
                        kind: "file".to_string(),
                    });
                }
            }
        }
    }

    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(DirContents {
        current_path: clean_current,
        parent_path,
        entries,
        drives: all_drives,
        cloud_folders: all_cloud,
        root_name,
        home_path: clean_home,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssetFileInfo {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub extension: String,
    pub size_bytes: u64,
    pub mtime_ms: u64,
}

#[tauri::command]
fn scan_library_folder(folder_path: String, extensions: Option<Vec<String>>) -> Result<Vec<AssetFileInfo>, String> {
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    let target = PathBuf::from(&folder_path);
    if !target.exists() || !target.is_dir() {
        return Ok(Vec::new());
    }

    let exts: Option<Vec<String>> = extensions.map(|list| {
        list.iter()
            .map(|e| e.to_lowercase().trim_start_matches('.').to_string())
            .collect()
    });

    let mut results = Vec::new();
    let mut stack = vec![(target.clone(), String::new())];

    while let Some((dir, rel_prefix)) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                if file_name.starts_with('.') {
                    continue;
                }

                let rel_path = if rel_prefix.is_empty() {
                    file_name.clone()
                } else {
                    format!("{}/{}", rel_prefix, file_name)
                };

                if path.is_dir() {
                    stack.push((path, rel_path));
                } else if path.is_file() {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .unwrap_or_default();

                    let matches = match &exts {
                        Some(allowed) => allowed.contains(&ext),
                        None => true,
                    };

                    if matches {
                        let metadata = entry.metadata().ok();
                        let size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                        let mtime_ms = metadata
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        results.push(AssetFileInfo {
                            name: file_name,
                            path: path.to_string_lossy().to_string(),
                            relative_path: rel_path,
                            extension: ext,
                            size_bytes,
                            mtime_ms,
                        });
                    }
                }
            }
        }
    }

    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(results)
}

#[tauri::command]
fn check_files_exist(paths: Vec<String>) -> Result<Vec<bool>, String> {
    use std::path::Path;
    let mut results = Vec::with_capacity(paths.len());
    for p in paths {
        let exists = Path::new(&p).exists();
        results.push(exists);
    }
    Ok(results)
}

#[tauri::command]
fn get_waveform_slice(
    state: State<'_, AppState>,
    start_frame: usize,
    end_frame: usize,
    num_points: usize
) -> Result<Vec<f32>, String> {
    let audio = {
        let active_opt = state.active_audio.lock().unwrap();
        active_opt.clone().ok_or_else(|| "No active track loaded".to_string())?
    };

    let channels = audio.channels;
    let total_frames = audio.channel_samples[0].len();

    let start = std::cmp::min(start_frame, total_frames);
    let end = std::cmp::min(end_frame, total_frames);
    
    if start >= end {
        return Ok(Vec::new());
    }

    let slice_len = end - start;
    let mut data = Vec::with_capacity(num_points);

    if slice_len <= num_points {
        for i in start..end {
            let mut sum = 0.0f32;
            for c in 0..channels {
                sum += audio.channel_samples[c][i];
            }
            data.push(sum / channels as f32);
        }
    } else {
        let chunk_size = slice_len as f64 / num_points as f64;
        for i in 0..num_points {
            let idx = start + (i as f64 * chunk_size) as usize;
            if idx >= end {
                break;
            }
            let mut sum = 0.0f32;
            for c in 0..channels {
                sum += audio.channel_samples[c][idx];
            }
            data.push(sum / channels as f32);
        }
    }

    Ok(data)
}

#[tauri::command]
fn get_raw_samples(
    state: State<'_, AppState>,
    start_frame: usize,
    count: usize
) -> Result<Vec<f32>, String> {
    let audio = {
        let active_opt = state.active_audio.lock().unwrap();
        active_opt.clone().ok_or_else(|| "No active track loaded".to_string())?
    };

    let channels = audio.channels;
    let total_frames = audio.channel_samples[0].len();

    let start = std::cmp::min(start_frame, total_frames);
    let end = std::cmp::min(start + count, total_frames);
    
    if start >= end {
        return Ok(Vec::new());
    }

    let len = end - start;
    let mut data = Vec::with_capacity(len);

    for i in start..end {
        let mut sum = 0.0f32;
        for c in 0..channels {
            sum += audio.channel_samples[c][i];
        }
        data.push(sum / channels as f32);
    }

    Ok(data)
}

#[tauri::command]
async fn preload_track(state: State<'_, AppState>, path: String) -> Result<TrackMetadata, String> {
    let (current_mtime, current_size) = get_file_mtime_and_size(&path);
    {
        let mut cache = state.track_cache.lock().unwrap();
        if let Some(cached) = cache.get(&path) {
            if cached.modified_time == current_mtime && cached.file_size == current_size {
                return Ok(cached.metadata.clone());
            }
        }
    }
    let p = path.clone();
    let target_sample_rate = state.shared_engine_state.device_sample_rate.load(std::sync::atomic::Ordering::SeqCst) as u32;
    let decoded_res = tauri::async_runtime::spawn_blocking(move || {
        let mut audio = decode_file(&p)?;
        let orig_sample_rate = audio.sample_rate;
        if target_sample_rate > 0 && audio.sample_rate != target_sample_rate {
            let resampled = resample_audio_channels(&audio.channel_samples, audio.sample_rate, target_sample_rate)?;
            audio.channel_samples = resampled;
            audio.sample_rate = target_sample_rate;
        }
        let arc = Arc::new(audio);
        let overview_peaks = compute_peaks(&arc, 1000);
        let pyramid_peaks = compute_pyramid_peaks(&arc, 32768);
        let metadata = TrackMetadata {
            duration_seconds: arc.duration_seconds,
            sample_rate: orig_sample_rate,
            channels: arc.channels,
            overview_peaks,
            pyramid_peaks,
        };
        Ok::<(Arc<DecodedAudio>, TrackMetadata), String>((arc, metadata))
    }).await;

    match decoded_res {
        Ok(Ok((audio_arc, metadata))) => {
            let mut cache = state.track_cache.lock().unwrap();
            let cached = Arc::new(CachedTrack {
                audio: audio_arc,
                metadata: metadata.clone(),
                modified_time: current_mtime,
                file_size: current_size,
            });
            cache.insert(path, cached);
            Ok(metadata)
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn load_track(state: State<'_, AppState>, path: String) -> Result<TrackMetadata, String> {
    let (current_mtime, current_size) = get_file_mtime_and_size(&path);
    
    // 1. Check LRU cache under brief lock
    let cached_opt = {
        let mut cache = state.track_cache.lock().unwrap();
        if let Some(cached) = cache.get(&path) {
            if cached.modified_time == current_mtime && cached.file_size == current_size {
                Some(cached)
            } else {
                None
            }
        } else {
            None
        }
    };

    let cached_track = match cached_opt {
        Some(track) => track,
        None => {
            // 2. Decode outside the lock on a worker thread to keep the main/UI thread responsive
            let p = path.clone();
            let target_sample_rate = state.shared_engine_state.device_sample_rate.load(std::sync::atomic::Ordering::SeqCst) as u32;
            let decoded_res = tauri::async_runtime::spawn_blocking(move || {
                let mut audio = decode_file(&p)?;
                let orig_sample_rate = audio.sample_rate;
                if target_sample_rate > 0 && audio.sample_rate != target_sample_rate {
                    let resampled = resample_audio_channels(&audio.channel_samples, audio.sample_rate, target_sample_rate)?;
                    audio.channel_samples = resampled;
                    audio.sample_rate = target_sample_rate;
                }
                let arc = Arc::new(audio);
                let overview_peaks = compute_peaks(&arc, 1000);
                let pyramid_peaks = compute_pyramid_peaks(&arc, 32768);
                let metadata = TrackMetadata {
                    duration_seconds: arc.duration_seconds,
                    sample_rate: orig_sample_rate,
                    channels: arc.channels,
                    overview_peaks,
                    pyramid_peaks,
                };
                Ok::<(Arc<DecodedAudio>, TrackMetadata), String>((arc, metadata))
            }).await;

            let (audio_arc, metadata) = match decoded_res {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e.to_string()),
            };

            let cached = Arc::new(CachedTrack {
                audio: audio_arc,
                metadata,
                modified_time: current_mtime,
                file_size: current_size,
            });

            // 3. Insert into LRU cache under brief lock
            let mut cache = state.track_cache.lock().unwrap();
            cache.insert(path, cached.clone());
            cached
        }
    };

    {
        let mut active_audio = state.active_audio.lock().unwrap();
        *active_audio = Some(cached_track.audio.clone());
    }

    state.command_bus.send(Command::LoadAudio(cached_track.audio.clone()))?;

    Ok(cached_track.metadata.clone())
}

#[tauri::command]
fn play(state: State<'_, AppState>) -> Result<(), String> {
    state.command_bus.send(Command::Play)
}

#[tauri::command]
fn pause(state: State<'_, AppState>) -> Result<(), String> {
    state.command_bus.send(Command::Pause)
}

#[tauri::command]
fn stop(state: State<'_, AppState>) -> Result<(), String> {
    state.command_bus.send(Command::Stop)
}

#[tauri::command]
fn seek(state: State<'_, AppState>, seconds: f64) -> Result<(), String> {
    state.command_bus.send(Command::Seek(Duration::from_secs_f64(seconds)))
}

#[tauri::command]
fn set_volume(state: State<'_, AppState>, volume: f32) -> Result<(), String> {
    state.command_bus.send(Command::SetVolume(volume))
}

#[tauri::command]
fn set_speed(state: State<'_, AppState>, speed: f32) -> Result<(), String> {
    state.command_bus.send(Command::SetTempo(speed))
}

#[tauri::command]
fn set_pitch(state: State<'_, AppState>, pitch: f32) -> Result<(), String> {
    state.command_bus.send(Command::SetPitch(pitch))
}

#[tauri::command]
fn set_eq(state: State<'_, AppState>, bass_db: f32, mid_db: f32, treble_db: f32) -> Result<(), String> {
    state.command_bus.send(Command::SetEq { bass_db, mid_db, treble_db })
}

#[tauri::command]
fn set_eq_bands(state: State<'_, AppState>, bands: Vec<trackhelm_engine::command::EqBand>) -> Result<(), String> {
    let mut fixed_bands = [trackhelm_engine::command::EqBand::default(); trackhelm_engine::command::MAX_EQ_BANDS];
    let count = std::cmp::min(bands.len(), trackhelm_engine::command::MAX_EQ_BANDS);
    for (i, band) in bands.iter().take(count).enumerate() {
        fixed_bands[i] = *band;
    }
    state.command_bus.send(Command::SetEqBands(fixed_bands, count))
}

#[tauri::command]
fn set_compressor(
    state: State<'_, AppState>,
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
    attack_ms: f32,
    release_ms: f32,
) -> Result<(), String> {
    state.command_bus.send(Command::SetCompressor {
        threshold_db,
        ratio,
        makeup_db,
        attack_ms,
        release_ms,
    })
}

#[tauri::command]
fn set_dual_compressor(
    state: State<'_, AppState>,
    stage1: trackhelm_engine::dsp::CompStageParams,
    stage2: trackhelm_engine::dsp::CompStageParams,
    routing: trackhelm_engine::dsp::CompRouting,
    parallel_blend: f32,
) -> Result<(), String> {
    state.command_bus.send(Command::SetDualCompressor {
        stage1,
        stage2,
        routing,
        parallel_blend,
    })
}

#[tauri::command]
fn set_regions(state: State<'_, AppState>, regions: Vec<trackhelm_engine::command::EngineRegion>) -> Result<(), String> {
    let mut fixed_regions = [trackhelm_engine::command::EngineRegion::default(); trackhelm_engine::command::MAX_ENGINE_REGIONS];
    let count = std::cmp::min(regions.len(), trackhelm_engine::command::MAX_ENGINE_REGIONS);
    for (i, reg) in regions.iter().take(count).enumerate() {
        fixed_regions[i] = *reg;
    }
    state.command_bus.send(Command::SetRegions(fixed_regions, count))
}

#[tauri::command]
fn set_volume_envelope(
    state: State<'_, AppState>,
    nodes: Vec<trackhelm_engine::EnvelopeNode>,
) -> Result<(), String> {
    let mut fixed_nodes = [trackhelm_engine::EnvelopeNode::default(); trackhelm_engine::MAX_ENVELOPE_NODES];
    let count = std::cmp::min(nodes.len(), trackhelm_engine::MAX_ENVELOPE_NODES);
    for (i, node) in nodes.iter().take(count).enumerate() {
        fixed_nodes[i] = *node;
    }
    state.command_bus.send(Command::SetVolumeEnvelope(fixed_nodes, count))
}

#[tauri::command]
fn get_playback_status(state: State<'_, AppState>) -> PlaybackStatus {
    let is_playing = state.shared_engine_state.is_playing.load(std::sync::atomic::Ordering::SeqCst);
    let current_frame = state.shared_engine_state.current_frame.load(std::sync::atomic::Ordering::SeqCst);
    
    // Read from the active audio lock first for accurate synchronous metadata
    let active_opt = state.active_audio.lock().unwrap();
    let (total_frames, sample_rate) = if let Some(audio) = active_opt.as_ref() {
        (audio.channel_samples[0].len(), audio.sample_rate as usize)
    } else {
        let total_frames = state.shared_engine_state.total_frames.load(std::sync::atomic::Ordering::SeqCst);
        let sample_rate = state.shared_engine_state.sample_rate.load(std::sync::atomic::Ordering::SeqCst);
        (total_frames, sample_rate)
    };

    let duration_seconds = if sample_rate > 0 {
        total_frames as f64 / sample_rate as f64
    } else {
        0.0
    };

    let current_time = if sample_rate > 0 {
        current_frame as f64 / sample_rate as f64
    } else {
        0.0
    };

    let progress = if total_frames > 0 {
        current_frame as f32 / total_frames as f32
    } else {
        0.0
    };

    let in_peak_l = state.shared_engine_state.in_peak_db_l.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    let in_peak_r = state.shared_engine_state.in_peak_db_r.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    let out_peak_l = state.shared_engine_state.out_peak_db_l.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    let out_peak_r = state.shared_engine_state.out_peak_db_r.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    let gr_stage1 = state.shared_engine_state.gr_stage1_db.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    let gr_stage2 = state.shared_engine_state.gr_stage2_db.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;

    PlaybackStatus {
        is_playing,
        current_time,
        duration_seconds,
        progress,
        in_peak_l,
        in_peak_r,
        out_peak_l,
        out_peak_r,
        gr_stage1,
        gr_stage2,
    }
}

fn compute_peaks(audio: &DecodedAudio, num_peaks: usize) -> Vec<f32> {
    let channels = audio.channels;
    let len = audio.channel_samples[0].len();
    let chunk_size = (len / num_peaks).max(1);

    let mut overview_peaks = Vec::with_capacity(num_peaks);

    for i in 0..num_peaks {
        let start = i * chunk_size;
        let end = std::cmp::min(start + chunk_size, len);
        if start >= len {
            overview_peaks.push(0.0);
            continue;
        }

        let mut mono_max = 0.0f32;
        for j in start..end {
            let mut sum = 0.0f32;
            for c in 0..channels {
                sum += audio.channel_samples[c][j];
            }
            let val = (sum / channels as f32).abs();
            if val > mono_max {
                mono_max = val;
            }
        }
        overview_peaks.push(mono_max);
    }
    overview_peaks
}

fn compute_pyramid_peaks(audio: &DecodedAudio, num_peaks: usize) -> Vec<f32> {
    let channels = audio.channels;
    let len = audio.channel_samples[0].len();
    let step = len as f64 / num_peaks as f64;

    let mut samples = Vec::with_capacity(num_peaks);

    for i in 0..num_peaks {
        let idx = (i as f64 * step) as usize;
        if idx >= len {
            samples.push(0.0);
            continue;
        }

        let mut sum = 0.0f32;
        for c in 0..channels {
            sum += audio.channel_samples[c][idx];
        }
        samples.push(sum / channels as f32);
    }
    samples
}

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

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug)]
pub struct AudioTagMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub grouping: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub comment: Option<String>,
    pub is_editable: bool,
}

#[tauri::command]
fn read_audio_metadata(path: String) -> Result<AudioTagMetadata, String> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, ItemKey};

    let tagged_file = match Probe::open(&path) {
        Ok(probe) => match probe.read() {
            Ok(tf) => tf,
            Err(_) => return Ok(AudioTagMetadata { is_editable: false, ..Default::default() }),
        },
        Err(_) => return Ok(AudioTagMetadata { is_editable: false, ..Default::default() }),
    };

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());

    let mut meta = AudioTagMetadata {
        is_editable: true,
        ..Default::default()
    };

    if let Some(tag) = tag {
        meta.title = tag.title().as_deref().map(|s| s.to_string());
        meta.artist = tag.artist().as_deref().map(|s| s.to_string());
        meta.album = tag.album().as_deref().map(|s| s.to_string());
        meta.genre = tag.genre().as_deref().map(|s| s.to_string());
        meta.year = tag.year();
        meta.track_number = tag.track();
        meta.comment = tag.comment().as_deref().map(|s| s.to_string());
        meta.grouping = tag.get_string(&ItemKey::ContentGroup).map(|s| s.to_string());
        meta.composer = tag.get_string(&ItemKey::Composer).map(|s| s.to_string());
    }

    Ok(meta)
}

#[tauri::command]
fn save_audio_metadata(path: String, metadata: AudioTagMetadata) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, ItemKey, Tag, TagExt};

    let mut tagged_file = Probe::open(&path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;

    let tag_type = tagged_file.primary_tag_type();
    let tag = match tagged_file.tag_mut(tag_type) {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.tag_mut(tag_type).ok_or("Failed to create tag")?
        }
    };

    if let Some(title) = metadata.title {
        tag.set_title(title);
    }
    if let Some(artist) = metadata.artist {
        tag.set_artist(artist);
    }
    if let Some(album) = metadata.album {
        tag.set_album(album);
    }
    if let Some(genre) = metadata.genre {
        tag.set_genre(genre);
    }
    if let Some(year) = metadata.year {
        tag.set_year(year);
    }
    if let Some(track_num) = metadata.track_number {
        tag.set_track(track_num);
    }
    if let Some(comment) = metadata.comment {
        tag.set_comment(comment);
    }
    if let Some(grouping) = metadata.grouping {
        tag.insert_text(ItemKey::ContentGroup, grouping);
    }
    if let Some(composer) = metadata.composer {
        tag.insert_text(ItemKey::Composer, composer);
    }

    tag.save_to_path(&path, WriteOptions::default()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn read_file_bytes(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| e.to_string())
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItemDto {
    pub name: String,
    pub path: String,
    pub duration: Option<f64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportAudioRequest {
    pub source_path: String,
    pub output_path: String,
    pub bit_depth: String, // "int16", "int24", "float32"
    pub range_start_seconds: Option<f64>,
    pub range_end_seconds: Option<f64>,
    pub pitch_semitones: f32,
    pub speed_multiplier: f32,
    pub volume_multiplier: f32,
    pub bake_pitch: bool,
    pub bake_speed: bool,
    pub bake_eq: bool,
    pub bake_compressor: bool,
    pub bake_cuts: bool,
    #[serde(default)]
    pub bake_envelope: bool,
    pub eq_bands: Vec<trackhelm_engine::command::EqBand>,
    pub comp_stage1: trackhelm_engine::dsp::CompStageParams,
    pub comp_stage2: trackhelm_engine::dsp::CompStageParams,
    pub comp_routing: trackhelm_engine::dsp::CompRouting,
    pub comp_parallel_blend: f32,
    pub regions: Vec<trackhelm_engine::command::EngineRegion>,
    #[serde(default)]
    pub envelope_nodes: Vec<trackhelm_engine::EnvelopeNode>,
    pub copy_metadata: bool,
}

#[tauri::command]
async fn export_audio_file(
    state: State<'_, AppState>,
    request: ExportAudioRequest,
) -> Result<String, String> {
    // 1. Get or decode the source audio
    let cached_audio = {
        let mut cache = state.track_cache.lock().unwrap();
        cache.get(&request.source_path).map(|c| c.audio.clone())
    };

    let audio_arc = if let Some(audio) = cached_audio {
        audio
    } else {
        let src_path = request.source_path.clone();
        let decoded = tauri::async_runtime::spawn_blocking(move || {
            trackhelm_engine::decoder::decode_file(&src_path)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

        Arc::new(decoded)
    };

    let bit_depth = match request.bit_depth.to_lowercase().as_str() {
        "int16" | "16" => trackhelm_engine::ExportBitDepth::Int16,
        "int24" | "24" => trackhelm_engine::ExportBitDepth::Int24,
        "float32" | "32" => trackhelm_engine::ExportBitDepth::Float32,
        _ => trackhelm_engine::ExportBitDepth::Int24,
    };

    let config = trackhelm_engine::ExportAudioConfig {
        output_path: request.output_path.clone(),
        bit_depth,
        range_start_seconds: request.range_start_seconds,
        range_end_seconds: request.range_end_seconds,
        pitch_semitones: request.pitch_semitones,
        speed_multiplier: request.speed_multiplier,
        volume_multiplier: request.volume_multiplier,
        bake_pitch: request.bake_pitch,
        bake_speed: request.bake_speed,
        bake_eq: request.bake_eq,
        bake_compressor: request.bake_compressor,
        bake_cuts: request.bake_cuts,
        bake_envelope: request.bake_envelope,
        eq_bands: request.eq_bands,
        comp_stage1: request.comp_stage1,
        comp_stage2: request.comp_stage2,
        comp_routing: request.comp_routing,
        comp_parallel_blend: request.comp_parallel_blend,
        regions: request.regions,
        envelope_nodes: request.envelope_nodes,
    };

    // 2. Run offline DSP render & encoding on blocking thread
    let out_path = request.output_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        trackhelm_engine::render_audio_export(&audio_arc, &config)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    // 3. Copy metadata tags if requested and source exists
    if request.copy_metadata {
        if let Ok(src_probe) = lofty::probe::Probe::open(&request.source_path) {
            if let Ok(src_file) = src_probe.read() {
                if let Some(src_tag) = src_file.primary_tag().or_else(|| src_file.first_tag()) {
                    let _ = src_tag.save_to_path(&out_path, lofty::config::WriteOptions::default());
                }
            }
        }
    }

    Ok(request.output_path)
}

#[tauri::command]
fn save_playlist_file(path: String, format: String, items: Vec<PlaylistItemDto>) -> Result<(), String> {
    match format.to_lowercase().as_str() {
        "m3u" | "m3u8" => {
            let mut content = String::from("#EXTM3U\n");
            for item in items {
                let duration_int = item.duration.unwrap_or(0.0).round() as i64;
                content.push_str(&format!("#EXTINF:{},{}\n", duration_int, item.name));
                content.push_str(&format!("{}\n", item.path));
            }
            std::fs::write(&path, content).map_err(|e| e.to_string())?;
        }
        "thset" | "json" => {
            #[derive(serde::Serialize)]
            struct ThSetFile {
                version: u32,
                items: Vec<PlaylistItemDto>,
            }
            let data = ThSetFile {
                version: 1,
                items,
            };
            let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
        }
        _ => return Err(format!("Unsupported playlist format: {}", format)),
    }
    Ok(())
}

#[tauri::command]
fn load_playlist_file(path: String) -> Result<Vec<PlaylistItemDto>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read playlist file: {}", e))?;
    let path_buf = std::path::PathBuf::from(&path);
    let parent_dir = path_buf.parent().unwrap_or(std::path::Path::new(""));

    let lower = path.to_lowercase();
    if lower.ends_with(".json") || lower.ends_with(".thset") {
        #[derive(serde::Deserialize)]
        struct ThSetFile {
            #[serde(default)]
            items: Vec<PlaylistItemDto>,
        }

        if let Ok(thset) = serde_json::from_str::<ThSetFile>(&content) {
            return Ok(thset.items);
        } else if let Ok(items) = serde_json::from_str::<Vec<PlaylistItemDto>>(&content) {
            return Ok(items);
        } else {
            return Err("Failed to parse JSON playlist".to_string());
        }
    }

    // M3U / M3U8 parser
    let mut items = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_duration: Option<f64> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("#EXTINF:") {
            let info = &trimmed[8..];
            if let Some((dur_str, title)) = info.split_once(',') {
                current_duration = dur_str.trim().parse::<f64>().ok();
                current_name = Some(title.trim().to_string());
            } else {
                current_name = Some(info.trim().to_string());
            }
        } else if !trimmed.starts_with('#') {
            // Audio file path
            let file_path = if std::path::Path::new(trimmed).is_absolute() {
                trimmed.to_string()
            } else {
                parent_dir.join(trimmed).to_string_lossy().to_string()
            };

            let name = current_name.take().unwrap_or_else(|| {
                std::path::Path::new(&file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&file_path)
                    .to_string()
            });

            items.push(PlaylistItemDto {
                name,
                path: file_path,
                duration: current_duration.take(),
            });
        }
    }

    Ok(items)
}

#[tauri::command]
fn broadcast_remote_state(state_json: String, state: State<'_, AppState>) {
    state.ws_state.broadcast(state_json);
}

#[tauri::command]
fn list_midi_devices(state: State<'_, AppState>) -> Vec<String> {
    state.midi_manager.list_ports()
}

#[tauri::command]
fn connect_midi_device(device_name: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    state.midi_manager.connect_port(app, &device_name)
}

#[derive(Clone, Serialize, Deserialize)]
struct GeneratedStemItem {
    name: String,
    path: String,
    role: String,
    #[serde(rename = "fileType")]
    file_type: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct UvrSeparationResult {
    success: bool,
    files: Vec<GeneratedStemItem>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct UvrProgressEvent {
    percent: u32,
    stage: String,
}

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
            // UF_DATALESS = 0x40000000 indicates an online-only cloud placeholder on APFS / FileProvider
            const UF_DATALESS: u32 = 0x4000_0000;
            if (flags & UF_DATALESS) != 0 {
                return false;
            }
            // If file is non-empty but has 0 blocks allocated, it is not downloaded
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
            // FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS = 0x00400000, FILE_ATTRIBUTE_OFFLINE = 0x00001000
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

#[tauri::command]
fn move_file_to_trash(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("File does not exist: {}", path));
    }

    trash::delete(p).map_err(|e| format!("Failed to move file to trash: {}", e))
}

const UVR_SEPARATOR_SCRIPT: &str = include_str!("../../scripts/uvr_separator.py");

#[tauri::command]
async fn run_uvr_separation(
    track_path: String,
    mode: String,
    output_dir: Option<String>,
    app: tauri::AppHandle,
) -> Result<UvrSeparationResult, String> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command as StdCommand, Stdio};

    let (script_path, python_path, project_dir) = {
        let mut candidate_roots = Vec::new();

        // 1. Tauri resource directory (for packaged app)
        if let Ok(res_dir) = app.path().resource_dir() {
            candidate_roots.push(res_dir.clone());
            candidate_roots.push(res_dir.join("scripts"));
        }

        // 2. Current working directory and its parent
        if let Ok(cwd) = std::env::current_dir() {
            candidate_roots.push(cwd.clone());
            if let Some(p) = cwd.parent() {
                candidate_roots.push(p.to_path_buf());
            }
        }

        // 3. Current executable ancestors
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidate_roots.push(exe_dir.to_path_buf());
                candidate_roots.push(exe_dir.join("scripts"));
                if let Some(p1) = exe_dir.parent() {
                    candidate_roots.push(p1.to_path_buf());
                    if let Some(p2) = p1.parent() {
                        candidate_roots.push(p2.to_path_buf());
                        if let Some(p3) = p2.parent() {
                            candidate_roots.push(p3.to_path_buf());
                        }
                    }
                }
            }
        }

        // 4. App local data directory
        if let Ok(data_dir) = app.path().app_data_dir() {
            candidate_roots.push(data_dir.clone());
            candidate_roots.push(data_dir.join("scripts"));
        }

        let mut found_script = None;
        let mut found_project = None;
        for root in &candidate_roots {
            let candidate1 = root.join("scripts/uvr_separator.py");
            if candidate1.exists() {
                found_script = Some(candidate1);
                found_project = Some(root.clone());
                break;
            }
            let candidate2 = root.join("uvr_separator.py");
            if candidate2.exists() {
                found_script = Some(candidate2);
                found_project = Some(root.clone());
                break;
            }
        }

        // If not found in any standard paths, automatically unpack embedded script to AppData
        if found_script.is_none() {
            if let Ok(data_dir) = app.path().app_data_dir() {
                let scripts_dir = data_dir.join("scripts");
                let _ = std::fs::create_dir_all(&scripts_dir);
                let target_script = scripts_dir.join("uvr_separator.py");
                if std::fs::write(&target_script, UVR_SEPARATOR_SCRIPT).is_ok() {
                    found_script = Some(target_script);
                    found_project = Some(data_dir);
                }
            }
        }

        // Fallback to temp dir if AppData write failed
        if found_script.is_none() {
            let temp_script = std::env::temp_dir().join("trackhelm_uvr_separator.py");
            if std::fs::write(&temp_script, UVR_SEPARATOR_SCRIPT).is_ok() {
                found_script = Some(temp_script);
                found_project = Some(std::env::temp_dir());
            }
        }

        let script = found_script.ok_or_else(|| {
            "Separation worker script not found in TrackHelm project directory".to_string()
        })?;
        let proj = found_project.unwrap_or_else(|| {
            script.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."))
        });

        let mut found_py = None;

        // Check local virtualenvs in candidate roots
        for root in &candidate_roots {
            #[cfg(target_os = "windows")]
            {
                let venv_py = root.join(".venv").join("Scripts").join("python.exe");
                if venv_py.exists() {
                    found_py = Some(venv_py);
                    break;
                }
                let venv_py2 = root.join(".venv_windows").join("Scripts").join("python.exe");
                if venv_py2.exists() {
                    found_py = Some(venv_py2);
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

        // On Windows, check standard Python installations with audio-separator / UVR support
        #[cfg(target_os = "windows")]
        if found_py.is_none() {
            let mut win_py_candidates = Vec::new();
            if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
                let base = std::path::PathBuf::from(local_app);
                win_py_candidates.push(base.join("Programs").join("Python").join("Python311").join("python.exe"));
                win_py_candidates.push(base.join("Programs").join("Python").join("Python312").join("python.exe"));
                win_py_candidates.push(base.join("Programs").join("Python").join("Python310").join("python.exe"));
            }
            for drive in &["C", "D", "E", "F"] {
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Program Files\Python311\python.exe", drive)));
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Program Files\Python312\python.exe", drive)));
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Program Files\Python310\python.exe", drive)));
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Python311\python.exe", drive)));
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Python312\python.exe", drive)));
                win_py_candidates.push(std::path::PathBuf::from(format!(r"{}:\Python310\python.exe", drive)));
            }

            for p in win_py_candidates {
                if p.exists() {
                    found_py = Some(p);
                    break;
                }
            }
        }

        // Check Windows `py` launcher
        #[cfg(target_os = "windows")]
        if found_py.is_none() {
            use std::os::windows::process::CommandExt;
            for ver in &["-3.11", "-3.12", "-3.10", "-3"] {
                let mut c = StdCommand::new("py");
                c.creation_flags(0x08000000);
                if let Ok(output) = c.arg(ver).arg("-c").arg("import sys; print(sys.executable)").output() {
                    if output.status.success() {
                        let py_exe = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !py_exe.is_empty() && std::path::Path::new(&py_exe).exists() {
                            found_py = Some(std::path::PathBuf::from(py_exe));
                            break;
                        }
                    }
                }
            }
        }

        // Fallback on Windows: where.exe python.exe (filtering out Inkscape)
        #[cfg(target_os = "windows")]
        if found_py.is_none() {
            use std::os::windows::process::CommandExt;
            let mut c = StdCommand::new("where.exe");
            c.creation_flags(0x08000000);
            if let Ok(output) = c.arg("python.exe").output() {
                if output.status.success() {
                    let out_str = String::from_utf8_lossy(&output.stdout);
                    for line in out_str.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.to_lowercase().contains("inkscape") {
                            found_py = Some(std::path::PathBuf::from(trimmed));
                            break;
                        }
                    }
                }
            }
        }

        #[cfg(target_os = "windows")]
        let py = found_py.unwrap_or_else(|| std::path::PathBuf::from("python.exe"));

        #[cfg(not(target_os = "windows"))]
        let py = found_py.unwrap_or_else(|| {
            let mac_candidates = [
                "/opt/homebrew/bin/python3",
                "/usr/local/bin/python3",
                "/usr/bin/python3",
            ];
            for c in &mac_candidates {
                let p = std::path::PathBuf::from(c);
                if p.exists() {
                    return p;
                }
            }
            std::path::PathBuf::from("python3")
        });

        (script, py, proj)
    };

    let mut cmd = StdCommand::new(&python_path);
    cmd.current_dir(&project_dir);
    cmd.arg(&script_path);
    cmd.arg("--input").arg(&track_path);
    cmd.arg("--mode").arg(&mode);

    if let Some(ref out) = output_dir {
        cmd.arg("--output-dir").arg(out);
    }

    // Configure model cache directory in app data / cache
    let models_cache = if let Ok(cache_dir) = app.path().app_cache_dir() {
        cache_dir.join("models_cache")
    } else if let Ok(data_dir) = app.path().app_data_dir() {
        data_dir.join("models_cache")
    } else {
        project_dir.join(".models_cache")
    };
    let _ = std::fs::create_dir_all(&models_cache);
    cmd.arg("--cache-dir").arg(&models_cache);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn separator: {}", e))?;
    let stdout = child.stdout.take().ok_or_else(|| "Failed to open stdout".to_string())?;
    let stderr = child.stderr.take();

    let stderr_thread = std::thread::spawn(move || {
        let mut err_msg = String::new();
        if let Some(pipe) = stderr {
            let reader = BufReader::new(pipe);
            for line in reader.lines() {
                if let Ok(l) = line {
                    if !err_msg.is_empty() {
                        err_msg.push('\n');
                    }
                    err_msg.push_str(&l);
                }
            }
        }
        err_msg
    });

    let app_handle = app.clone();
    let reader = BufReader::new(stdout);

    let mut result = UvrSeparationResult {
        success: false,
        files: Vec::new(),
        error: None,
    };

    for line in reader.lines() {
        if let Ok(line_str) = line {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line_str) {
                if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
                    match t {
                        "progress" => {
                            let percent = val.get("percent").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let stage = val.get("stage").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let _ = app_handle.emit("uvr-progress", UvrProgressEvent { percent, stage });
                        }
                        "complete" => {
                            if let Some(files_val) = val.get("files").and_then(|v| v.as_array()) {
                                for f in files_val {
                                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let role = f.get("role").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let file_type = f.get("fileType").and_then(|v| v.as_str()).unwrap_or("audio").to_string();
                                    result.files.push(GeneratedStemItem {
                                        name,
                                        path,
                                        role,
                                        file_type,
                                    });
                                }
                            }
                            result.success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                        }
                        "error" => {
                            let msg = val.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error").to_string();
                            result.error = Some(msg);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let stderr_output = stderr_thread.join().unwrap_or_default();
    let status = child.wait().map_err(|e| format!("Child wait error: {}", e))?;
    if !stderr_output.trim().is_empty() {
        eprintln!("[uvr stderr]:\n{}", stderr_output);
    }
    if !status.success() && result.error.is_none() {
        let lines: Vec<&str> = stderr_output.lines().collect();
        let detail = if let Some(idx) = lines.iter().position(|l| l.contains("Traceback (most recent call last):")) {
            lines[idx..].join("\n").trim().to_string()
        } else {
            let filtered: Vec<&str> = lines
                .into_iter()
                .filter(|l| {
                    let trimmed = l.trim();
                    let lower = trimmed.to_lowercase();
                    !lower.contains("- info -")
                        && !lower.contains("- warning -")
                        && !trimmed.contains("%|")
                        && !lower.contains("downloading")
                        && !lower.contains("cuda-executionprovider")
                        && !lower.contains("failed to load cublas")
                        && !lower.contains("failed to load cufft")
                        && !lower.contains("failed to load cudart")
                        && !lower.contains("please follow https://onnxruntime.ai")
                        && !trimmed.is_empty()
                })
                .collect();
            if !filtered.is_empty() {
                filtered.join("\n")
            } else {
                format!("Separation process exited with code {:?}", status.code())
            }
        };
        result.error = Some(detail);
    }

    if result.success {
        let _ = app.emit("uvr-complete", &result);
        Ok(result)
    } else {
        let err = result.error.clone().unwrap_or_else(|| "Separation failed".to_string());
        let _ = app.emit("uvr-error", &err);
        Err(err)
    }
}


fn create_app_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_menu = Submenu::with_items(
        app,
        "TrackHelm",
        true,
        &[
            &MenuItem::with_id(app, "open_about", "About TrackHelm...", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open_preferences", "Preferences...", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, "open_file", "Open Audio Track...", true, Some("CmdOrCtrl+O"))?,
            &MenuItem::with_id(app, "open_alternate", "Open Alternate Track...", true, Some("CmdOrCtrl+Alt+O"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open_playlist", "Open Playlist / Set...", true, Some("CmdOrCtrl+Shift+O"))?,
            &MenuItem::with_id(app, "save_playlist", "Save Playlist / Set...", true, Some("CmdOrCtrl+S"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "export_audio", "Export Audio File...", true, Some("CmdOrCtrl+Shift+E"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
            &PredefinedMenuItem::quit(app, Some("Quit"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &MenuItem::with_id(app, "add_marker", "Add Landmark / Marker", true, Some("M"))?,
            &MenuItem::with_id(app, "create_region", "Create Region from Selection", true, Some("R"))?,
            &MenuItem::with_id(app, "toggle_loop", "Toggle Loop Region", true, Some("L"))?,
            &MenuItem::with_id(app, "toggle_cut", "Toggle Cut Region", true, Some("X"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let playback_menu = Submenu::with_items(
        app,
        "Playback",
        true,
        &[
            &MenuItem::with_id(app, "play_pause", "Play / Pause", true, Some("Space"))?,
            &MenuItem::with_id(app, "stop", "Stop & Return to Start", true, Some("Enter"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "prev_marker", "Previous Marker", true, Some("Left"))?,
            &MenuItem::with_id(app, "next_marker", "Next Marker", true, Some("Right"))?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &MenuItem::with_id(app, "tab_notes", "Notes Tab", true, Some("CmdOrCtrl+1"))?,
            &MenuItem::with_id(app, "tab_lyrics", "Lyrics Tab", true, Some("CmdOrCtrl+2"))?,
            &MenuItem::with_id(app, "tab_metadata", "Metadata Tab", true, Some("CmdOrCtrl+3"))?,
            &MenuItem::with_id(app, "tab_files", "Files & Alternate Takes Tab", true, Some("CmdOrCtrl+4"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open_remotes", "Show Control & Remotes...", true, Some("CmdOrCtrl+Shift+R"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;

    let help_menu = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &MenuItem::with_id(app, "open_about", "About TrackHelm...", true, None::<&str>)?,
        ],
    )?;

    Menu::with_items(app, &[
        &app_menu,
        &file_menu,
        &edit_menu,
        &playback_menu,
        &view_menu,
        &help_menu,
    ])
}

fn main() {
    let (mut engine, command_bus, shared_state) = trackhelm_engine::AudioEngine::new();

    if let Err(e) = engine.start() {
        eprintln!("Audio engine failed to start: {}", e);
    }

    let ws_state = Arc::new(control::websocket::WebSocketServerState::new());
    let midi_manager = Arc::new(control::midi::MidiManager::new());
    let ws_state_clone = ws_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState {
            command_bus,
            shared_engine_state: shared_state,
            active_audio: Mutex::new(None),
            track_cache: Mutex::new(LruTrackCache::new(6)),
            ws_state,
            midi_manager,
        })
        .setup(move |app| {
            let menu = create_app_menu(app.handle())?;
            app.set_menu(menu)?;
            app.on_menu_event(|app_handle, event| {
                let id_str = event.id().as_ref().to_string();
                let _ = app_handle.emit("menu-action", id_str);
            });

            // Start WebSocket server on default port 4545 (ws://0.0.0.0:4545)
            control::websocket::start_websocket_server(app.handle().clone(), 4545, ws_state_clone);

            // Start OSC server on UDP port 4546
            control::osc::start_osc_server(app.handle().clone(), 4546);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_track,
            preload_track,
            play,
            pause,
            stop,
            seek,
            set_volume,
            set_speed,
            set_pitch,
            set_eq,
            set_eq_bands,
            set_compressor,
            set_dual_compressor,
            set_regions,
            set_volume_envelope,
            get_playback_status,
            read_dir,
            get_waveform_slice,
            get_raw_samples,
            get_cloud_folders,
            open_file_external,
            read_audio_metadata,
            save_audio_metadata,
            read_file_bytes,
            export_audio_file,
            save_playlist_file,
            load_playlist_file,
            broadcast_remote_state,
            list_midi_devices,
            connect_midi_device,
            scan_library_folder,
            check_files_exist,
            is_file_downloaded,
            move_file_to_trash,
            run_uvr_separation
        ])

        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
