#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use trackhelm_engine::{Command, CommandBus, SharedEngineState, DecodedAudio, decode_file};

struct AppState {
    command_bus: CommandBus,
    shared_engine_state: Arc<SharedEngineState>,
    active_audio: Mutex<Option<Arc<DecodedAudio>>>,
}

#[derive(serde::Serialize)]
struct TrackMetadata {
    duration_seconds: f64,
    sample_rate: u32,
    channels: usize,
    peaks: Vec<f32>,
}

#[derive(serde::Serialize)]
struct PlaybackStatus {
    is_playing: bool,
    current_time: f64,
    duration_seconds: f64,
    progress: f32,
}

#[derive(serde::Serialize)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
    size_bytes: u64,
}

#[derive(serde::Serialize)]
struct DirContents {
    current_path: String,
    parent_path: Option<String>,
    entries: Vec<DirEntry>,
}

#[tauri::command]
fn read_dir(path: Option<String>) -> Result<DirContents, String> {
    use std::path::PathBuf;
    
    let target_path = match path {
        Some(p) => {
            if p.starts_with('~') {
                let home = dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())?;
                if p.len() > 2 {
                    home.join(&p[2..]) // Skip "~/"
                } else {
                    home
                }
            } else {
                PathBuf::from(p)
            }
        }
        None => dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())?,
    };

    let canonical = target_path.canonicalize()
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    let parent_path = canonical.parent().map(|p| p.to_string_lossy().to_string());

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

            let path_str = entry.path().to_string_lossy().to_string();
            let size_bytes = metadata.map(|m| m.len()).unwrap_or(0);

            if is_dir {
                entries.push(DirEntry {
                    name,
                    path: path_str,
                    is_dir: true,
                    size_bytes,
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
                        path: path_str,
                        is_dir: false,
                        size_bytes,
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
        current_path: canonical.to_string_lossy().to_string(),
        parent_path,
        entries,
    })
}

#[tauri::command]
fn get_cloud_folders() -> Result<Vec<DirEntry>, String> {
    let mut folders = Vec::new();
    let home = dirs::home_dir().ok_or_else(|| "Home directory not found".to_string())?;
    
    // Scan Library/CloudStorage
    let cloud_storage = home.join("Library/CloudStorage");
    if cloud_storage.exists() && cloud_storage.is_dir() {
        if let Ok(entries) = std::fs::read_dir(cloud_storage) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.starts_with('.') {
                            folders.push(DirEntry {
                                name,
                                path: path.to_string_lossy().to_string(),
                                is_dir: true,
                                size_bytes: 0,
                            });
                        }
                    }
                }
            }
        }
    }

    // Scan common legacy folders direct in home folder
    let legacy_folders = vec!["Dropbox", "OneDrive", "Google Drive"];
    for name in legacy_folders {
        let path = home.join(name);
        if path.exists() && path.is_dir() {
            let path_str = path.to_string_lossy().to_string();
            if !folders.iter().any(|f| f.path == path_str) {
                folders.push(DirEntry {
                    name: name.to_string(),
                    path: path_str,
                    is_dir: true,
                    size_bytes: 0,
                });
            }
        }
    }

    Ok(folders)
}

#[tauri::command]
fn get_waveform_slice(
    state: State<'_, AppState>,
    start_frame: usize,
    end_frame: usize,
    num_points: usize
) -> Result<Vec<f32>, String> {
    let active_opt = state.active_audio.lock().unwrap();
    let audio = active_opt.as_ref().ok_or_else(|| "No active track loaded".to_string())?;

    let channel_data = &audio.channel_samples[0]; // Mono/first channel peaks
    let total_frames = channel_data.len();

    let start = std::cmp::min(start_frame, total_frames);
    let end = std::cmp::min(end_frame, total_frames);
    
    if start >= end {
        return Ok(Vec::new());
    }

    let slice_len = end - start;
    let mut result = Vec::new();

    if slice_len <= num_points {
        // Sample level: return raw values
        for i in start..end {
            result.push(channel_data[i]);
        }
    } else {
        // Downsample slice
        let chunk_size = slice_len / num_points;
        for i in 0..num_points {
            let chunk_start = start + i * chunk_size;
            let chunk_end = std::cmp::min(chunk_start + chunk_size, end);
            if chunk_start >= chunk_end {
                break;
            }
            let mut max_val: f32 = 0.0;
            for j in chunk_start..chunk_end {
                let val = channel_data[j].abs();
                if val > max_val {
                    max_val = val;
                }
            }
            result.push(max_val);
        }
    }

    Ok(result)
}

#[tauri::command]
fn load_track(state: State<'_, AppState>, path: String) -> Result<TrackMetadata, String> {
    println!("Loading track: {}", path);
    let audio = decode_file(&path)?;
    let duration_seconds = audio.duration_seconds;
    let sample_rate = audio.sample_rate;
    let channels = audio.channels;

    let peaks = compute_peaks(&audio, 1000);
    let audio_arc = Arc::new(audio);

    let mut active_audio = state.active_audio.lock().unwrap();
    *active_audio = Some(audio_arc.clone());

    state.command_bus.send(Command::LoadAudio(audio_arc))?;

    Ok(TrackMetadata {
        duration_seconds,
        sample_rate,
        channels,
        peaks,
    })
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
fn get_playback_status(state: State<'_, AppState>) -> PlaybackStatus {
    let is_playing = state.shared_engine_state.is_playing.load(std::sync::atomic::Ordering::SeqCst);
    let current_frame = state.shared_engine_state.current_frame.load(std::sync::atomic::Ordering::SeqCst);
    let total_frames = state.shared_engine_state.total_frames.load(std::sync::atomic::Ordering::SeqCst);
    let sample_rate = state.shared_engine_state.sample_rate.load(std::sync::atomic::Ordering::SeqCst);

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

    PlaybackStatus {
        is_playing,
        current_time,
        duration_seconds,
        progress,
    }
}

fn compute_peaks(audio: &DecodedAudio, num_peaks: usize) -> Vec<f32> {
    let mut peaks = Vec::with_capacity(num_peaks);
    let channels = audio.channels;
    let len = audio.channel_samples[0].len();
    let chunk_size = (len / num_peaks).max(1);

    for i in 0..num_peaks {
        let start = i * chunk_size;
        let end = std::cmp::min(start + chunk_size, len);
        if start >= len {
            peaks.push(0.0);
            continue;
        }

        let mut max_val = 0.0f32;
        for c in 0..channels {
            for sample_idx in start..end {
                let val = audio.channel_samples[c][sample_idx].abs();
                if val > max_val {
                    max_val = val;
                }
            }
        }
        peaks.push(max_val);
    }
    peaks
}

fn main() {
    let (mut engine, command_bus, shared_state) = trackhelm_engine::AudioEngine::new();

    std::thread::spawn(move || {
        if let Err(e) = engine.start() {
            eprintln!("Audio engine failed to start: {}", e);
            return;
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState {
            command_bus,
            shared_engine_state: shared_state,
            active_audio: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            load_track,
            play,
            pause,
            stop,
            seek,
            set_volume,
            get_playback_status,
            read_dir,
            get_waveform_slice,
            get_cloud_folders
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
