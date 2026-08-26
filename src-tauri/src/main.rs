#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{State, Emitter};
use tauri::menu::{Menu, MenuItem, Submenu, PredefinedMenuItem};
use trackhelm_engine::{Command, CommandBus, SharedEngineState, DecodedAudio, decode_file};
use lofty::prelude::*;

mod control;

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
    pub eq_bands: Vec<trackhelm_engine::command::EqBand>,
    pub comp_stage1: trackhelm_engine::dsp::CompStageParams,
    pub comp_stage2: trackhelm_engine::dsp::CompStageParams,
    pub comp_routing: trackhelm_engine::dsp::CompRouting,
    pub comp_parallel_blend: f32,
    pub regions: Vec<trackhelm_engine::command::EngineRegion>,
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
        eq_bands: request.eq_bands,
        comp_stage1: request.comp_stage1,
        comp_stage2: request.comp_stage2,
        comp_routing: request.comp_routing,
        comp_parallel_blend: request.comp_parallel_blend,
        regions: request.regions,
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

fn create_app_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
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

    Menu::with_items(app, &[
        &file_menu,
        &edit_menu,
        &playback_menu,
        &view_menu,
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
            connect_midi_device
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
