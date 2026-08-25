#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use trackhelm_engine::{Command, CommandBus, SharedEngineState, DecodedAudio, decode_file};

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
    track_cache: Mutex<HashMap<String, Arc<CachedTrack>>>,
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
    
    let home = dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())?;
    
    let target_path = match path {
        Some(p) => {
            if p.starts_with('~') {
                if p.len() > 2 {
                    home.join(&p[2..]) // Skip "~/"
                } else {
                    home.clone()
                }
            } else {
                PathBuf::from(p)
            }
        }
        None => home.clone(),
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

    // Inject CloudStorage folder and cloud subfolders if we are in the home directory
    let is_home = canonical == home.canonicalize().unwrap_or_else(|_| home.clone());
    if is_home {
        let cloud_storage = home.join("Library/CloudStorage");
        if cloud_storage.exists() && cloud_storage.is_dir() {
            // 1. Inject cloud subfolders (e.g. Dropbox-Personal, GoogleDrive, OneDrive)
            if let Ok(sub_entries) = std::fs::read_dir(&cloud_storage) {
                for sub_entry in sub_entries {
                    if let Ok(sub_entry) = sub_entry {
                        let sub_metadata = sub_entry.metadata().ok();
                        let sub_is_dir = sub_metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                        if sub_is_dir {
                            let sub_name = sub_entry.file_name().to_string_lossy().to_string();
                            if !sub_name.starts_with('.') {
                                let sub_path_str = sub_entry.path().to_string_lossy().to_string();
                                // Avoid duplicate entry additions
                                if !entries.iter().any(|e| e.name == sub_name) {
                                    entries.push(DirEntry {
                                        name: sub_name,
                                        path: sub_path_str,
                                        is_dir: true,
                                        size_bytes: 0,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            
            // 2. Inject the CloudStorage folder itself
            if !entries.iter().any(|e| e.name == "CloudStorage") {
                entries.push(DirEntry {
                    name: "CloudStorage".to_string(),
                    path: cloud_storage.to_string_lossy().to_string(),
                    is_dir: true,
                    size_bytes: 0,
                });
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
    let active_opt = state.active_audio.lock().unwrap();
    let audio = active_opt.as_ref().ok_or_else(|| "No active track loaded".to_string())?;

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
        let cache = state.track_cache.lock().unwrap();
        if let Some(cached) = cache.get(&path) {
            if cached.modified_time == current_mtime && cached.file_size == current_size {
                return Ok(cached.metadata.clone());
            }
        }
    }
    let p = path.clone();
    let decoded_res = tauri::async_runtime::spawn_blocking(move || {
        let audio = decode_file(&p)?;
        let arc = Arc::new(audio);
        let overview_peaks = compute_peaks(&arc, 1000);
        let pyramid_peaks = compute_pyramid_peaks(&arc, 32768);
        let metadata = TrackMetadata {
            duration_seconds: arc.duration_seconds,
            sample_rate: arc.sample_rate,
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
fn load_track(state: State<'_, AppState>, path: String) -> Result<TrackMetadata, String> {
    let (current_mtime, current_size) = get_file_mtime_and_size(&path);
    let cached_track = {
        let mut cache = state.track_cache.lock().unwrap();
        if let Some(cached) = cache.get(&path) {
            if cached.modified_time == current_mtime && cached.file_size == current_size {
                cached.clone()
            } else {
                // File modified externally -> re-decode fresh
                let audio = decode_file(&path)?;
                let arc = Arc::new(audio);
                let overview_peaks = compute_peaks(&arc, 1000);
                let pyramid_peaks = compute_pyramid_peaks(&arc, 32768);
                let metadata = TrackMetadata {
                    duration_seconds: arc.duration_seconds,
                    sample_rate: arc.sample_rate,
                    channels: arc.channels,
                    overview_peaks,
                    pyramid_peaks,
                };
                let cached = Arc::new(CachedTrack {
                    audio: arc,
                    metadata,
                    modified_time: current_mtime,
                    file_size: current_size,
                });
                cache.insert(path.clone(), cached.clone());
                cached
            }
        } else {
            let audio = decode_file(&path)?;
            let arc = Arc::new(audio);
            let overview_peaks = compute_peaks(&arc, 1000);
            let pyramid_peaks = compute_pyramid_peaks(&arc, 32768);
            let metadata = TrackMetadata {
                duration_seconds: arc.duration_seconds,
                sample_rate: arc.sample_rate,
                channels: arc.channels,
                overview_peaks,
                pyramid_peaks,
            };
            let cached = Arc::new(CachedTrack {
                audio: arc,
                metadata,
                modified_time: current_mtime,
                file_size: current_size,
            });
            cache.insert(path.clone(), cached.clone());
            cached
        }
    };

    let mut active_audio = state.active_audio.lock().unwrap();
    *active_audio = Some(cached_track.audio.clone());

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

    PlaybackStatus {
        is_playing,
        current_time,
        duration_seconds,
        progress,
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
            track_cache: Mutex::new(HashMap::new()),
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
            get_playback_status,
            read_dir,
            get_waveform_slice,
            get_raw_samples,
            get_cloud_folders,
            open_file_external,
            read_audio_metadata,
            save_audio_metadata,
            read_file_bytes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
