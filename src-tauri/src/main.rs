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
            get_playback_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
