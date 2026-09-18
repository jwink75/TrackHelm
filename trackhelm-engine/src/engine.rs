use std::sync::atomic::{AtomicBool, AtomicUsize, AtomicI32, Ordering};
use std::sync::Arc;
use crate::command::{Command, CommandBus};
use crate::decoder::DecodedAudio;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct SharedEngineState {
    pub is_playing: Arc<AtomicBool>,
    pub current_frame: Arc<AtomicUsize>,
    pub total_frames: Arc<AtomicUsize>,
    pub sample_rate: Arc<AtomicUsize>,
    pub device_sample_rate: Arc<AtomicUsize>,
    pub volume_raw: Arc<AtomicUsize>, // Volume scaled by 1000 (e.g. 1.0 -> 1000)
    pub in_peak_db_l: Arc<AtomicI32>,  // (db * 100.0) as i32, default -6000 (-60.0 dB)
    pub in_peak_db_r: Arc<AtomicI32>,
    pub out_peak_db_l: Arc<AtomicI32>,
    pub out_peak_db_r: Arc<AtomicI32>,
    pub gr_stage1_db: Arc<AtomicI32>,  // (abs_gr_db * 100.0) as i32, default 0
    pub gr_stage2_db: Arc<AtomicI32>,
    pub background_tracks_count: Arc<AtomicUsize>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioDeviceItem {
    pub name: String,
    pub is_default: bool,
    pub sample_rate: u32,
    pub channels: u16,
}

enum HostCommand {
    SetDevice {
        name: Option<String>,
        reply: crossbeam_channel::Sender<Result<AudioDeviceItem, String>>,
    },
    GetDevice {
        reply: crossbeam_channel::Sender<Option<String>>,
    },
}

pub struct AudioEngine {
    host_sender: crossbeam_channel::Sender<HostCommand>,
}

pub fn list_output_devices() -> Vec<AudioDeviceItem> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let mut devices = Vec::new();

    if let Ok(dev_iter) = host.output_devices() {
        for dev in dev_iter {
            if let Ok(name) = dev.name() {
                let is_default = default_name.as_deref() == Some(&name);
                let (sample_rate, channels) = dev.default_output_config()
                    .map(|c| (c.sample_rate().0, c.channels()))
                    .unwrap_or((44100, 2));

                devices.push(AudioDeviceItem {
                    name,
                    is_default,
                    sample_rate,
                    channels,
                });
            }
        }
    }
    devices
}

struct BackgroundVoice {
    audio: Arc<DecodedAudio>,
    playback_frame: usize,
    speed: f32,
    pitch: f32,
    stretch: signalsmith_stretch_rs::SignalsmithStretch,
    stretch_channels: usize,
    biquads_pool: Vec<crate::dsp::Biquad>,
    active_biquad_count: usize,
    eq_active: bool,
    dual_compressor: crate::dsp::DualCompressor,
    volume: f32,
    envelope_nodes: [crate::command::EnvelopeNode; crate::command::MAX_ENVELOPE_NODES],
    envelope_nodes_count: usize,
    active_regions: [crate::command::EngineRegion; crate::command::MAX_ENGINE_REGIONS],
    active_region_count: usize,
    in_scratch: Vec<Vec<f32>>,
    out_scratch: Vec<Vec<f32>>,
}

fn render_single_voice(
    audio: &DecodedAudio,
    playback_frame: &mut usize,
    speed: f32,
    pitch: f32,
    stretch: &mut signalsmith_stretch_rs::SignalsmithStretch,
    stretch_channels: usize,
    biquads_pool: &mut [crate::dsp::Biquad],
    active_biquad_count: usize,
    eq_active: bool,
    dual_compressor: &mut crate::dsp::DualCompressor,
    volume: f32,
    envelope_nodes: &[crate::command::EnvelopeNode],
    active_regions: &[crate::command::EngineRegion],
    device_sample_rate: f64,
    output_channels: usize,
    num_out_frames: usize,
    in_scratch: &mut [Vec<f32>],
    out_scratch: &mut [Vec<f32>],
    dest: &mut [f32],
) -> bool {
    let audio_len = audio.channel_samples[0].len();
    let audio_channels = audio.channels;
    let frame_rate = audio.sample_rate as f64;

    // Region handling: Check for Cut skip and Loop wrap
    let current_sec = *playback_frame as f64 / frame_rate;
    for reg in active_regions {
        if reg.is_cut && current_sec >= reg.start_seconds && current_sec < reg.end_seconds {
            *playback_frame = (reg.end_seconds * frame_rate) as usize;
            break;
        } else if reg.is_loop && current_sec >= reg.end_seconds {
            *playback_frame = (reg.start_seconds * frame_rate) as usize;
            break;
        }
    }

    if *playback_frame >= audio_len {
        for s in dest.iter_mut().take(num_out_frames * output_channels) {
            *s = 0.0;
        }
        return true;
    }

    const MAX_BUFFER_FRAMES: usize = 16384;
    const MAX_CHANNELS: usize = 8;
    let safe_out_frames = std::cmp::min(num_out_frames, MAX_BUFFER_FRAMES);
    let num_in_frames = ((safe_out_frames as f32) * speed).round() as usize;
    let safe_in_frames = std::cmp::min(num_in_frames, MAX_BUFFER_FRAMES);

    for i in 0..safe_in_frames {
        let curr_f = *playback_frame + i;
        if curr_f < audio_len {
            for ch in 0..stretch_channels {
                in_scratch[ch][i] = audio.channel_samples[ch % audio_channels][curr_f];
            }
        } else {
            for ch in 0..stretch_channels {
                in_scratch[ch][i] = 0.0;
            }
        }
    }

    let mut in_slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
    for ch in 0..stretch_channels {
        in_slices[ch] = &in_scratch[ch][..safe_in_frames];
    }

    let mut out_slices: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|ch| {
        if ch < stretch_channels {
            unsafe {
                std::slice::from_raw_parts_mut(out_scratch[ch].as_mut_ptr(), safe_out_frames)
            }
        } else {
            &mut [][..]
        }
    });

    let is_modulating = pitch.abs() > 0.001 || (speed - 1.0).abs() > 0.001;
    if is_modulating {
        stretch.process(&in_slices[..stretch_channels], &mut out_slices[..stretch_channels]);
    } else {
        for ch in 0..stretch_channels {
            out_scratch[ch][..safe_out_frames].copy_from_slice(&in_scratch[ch][..safe_out_frames]);
        }
    }

    // Apply track volume & volume envelope PRE-EQ and PRE-COMPRESSOR
    for frame_idx in 0..safe_out_frames {
        let frame_time_sec = (*playback_frame + ((frame_idx as f32 * speed) as usize)) as f64 / device_sample_rate;
        let env_gain = if !envelope_nodes.is_empty() {
            crate::command::interpolate_envelope(envelope_nodes, frame_time_sec)
        } else {
            1.0
        };
        let total_pre_gain = volume * env_gain;

        for out_c in 0..output_channels {
            let in_c = out_c % stretch_channels;
            dest[frame_idx * output_channels + out_c] = out_scratch[in_c][frame_idx] * total_pre_gain;
        }
    }

    for frame_idx in safe_out_frames..num_out_frames {
        for out_c in 0..output_channels {
            dest[frame_idx * output_channels + out_c] = 0.0;
        }
    }

    // Apply High-Quality Biquad EQ Filters
    if eq_active && active_biquad_count > 0 {
        for frame_idx in 0..num_out_frames {
            for ch in 0..output_channels {
                let idx = frame_idx * output_channels + ch;
                let mut s = dest[idx];
                for b in &mut biquads_pool[..active_biquad_count] {
                    s = b.process_sample(ch, s);
                }
                dest[idx] = s;
            }
        }
    }

    // Apply Dual-Stage Dynamic Compressor
    if !dual_compressor.is_bypassed() {
        for frame_idx in 0..num_out_frames {
            let left_idx = frame_idx * output_channels;
            let right_idx = if output_channels > 1 { left_idx + 1 } else { left_idx };
            let (l, r) = dual_compressor.process_stereo_frame(dest[left_idx], dest[right_idx]);
            dest[left_idx] = l;
            if output_channels > 1 {
                dest[right_idx] = r;
            }
        }
    }

    *playback_frame += safe_in_frames;
    *playback_frame >= audio_len
}

impl AudioEngine {
    pub fn new() -> (Self, CommandBus, Arc<SharedEngineState>) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let command_bus = CommandBus::new(sender);

        let shared_state = Arc::new(SharedEngineState {
            is_playing: Arc::new(AtomicBool::new(false)),
            current_frame: Arc::new(AtomicUsize::new(0)),
            total_frames: Arc::new(AtomicUsize::new(0)),
            sample_rate: Arc::new(AtomicUsize::new(44100)),
            device_sample_rate: Arc::new(AtomicUsize::new(48000)),
            volume_raw: Arc::new(AtomicUsize::new(1000)), // default 1.0 volume
            in_peak_db_l: Arc::new(AtomicI32::new(-6000)),
            in_peak_db_r: Arc::new(AtomicI32::new(-6000)),
            out_peak_db_l: Arc::new(AtomicI32::new(-6000)),
            out_peak_db_r: Arc::new(AtomicI32::new(-6000)),
            gr_stage1_db: Arc::new(AtomicI32::new(0)),
            gr_stage2_db: Arc::new(AtomicI32::new(0)),
            background_tracks_count: Arc::new(AtomicUsize::new(0)),
        });

        let (host_sender, host_receiver) = crossbeam_channel::unbounded::<HostCommand>();
        let cmd_rx_clone = receiver.clone();
        let thread_shared_state = shared_state.clone();

        std::thread::spawn(move || {
            let mut active_stream: Option<cpal::Stream>;
            let mut current_device_name: Option<String> = None;

            while let Ok(cmd) = host_receiver.recv() {
                match cmd {
                    HostCommand::SetDevice { name, reply } => {
                        active_stream = None;

                        let res = (|| -> Result<AudioDeviceItem, String> {
                            let host = cpal::default_host();
                            let (target_device, is_default) = match name.as_deref() {
                                Some(d_name) => {
                                    let mut found = None;
                                    if let Ok(devices) = host.output_devices() {
                                        for d in devices {
                                            if let Ok(n) = d.name() {
                                                if n == d_name {
                                                    found = Some(d);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    let dev = found.ok_or_else(|| format!("Audio device '{}' not found", d_name))?;
                                    let is_def = host.default_output_device().and_then(|d| d.name().ok()).as_deref() == Some(d_name);
                                    (dev, is_def)
                                }
                                None => {
                                    let dev = host.default_output_device().ok_or_else(|| "No default audio output device found".to_string())?;
                                    (dev, true)
                                }
                            };

                            let dev_name = target_device.name().unwrap_or_else(|_| "Unknown Device".to_string());
                            let config = target_device.default_output_config()
                                .map_err(|e| format!("Failed to get output config for '{}': {}", dev_name, e))?;

                            let sample_rate = config.sample_rate().0;
                            let channels = config.channels();

                            log::info!("Initializing audio output device: {} ({:?})", dev_name, config);

                            let stream = Self::build_stream(
                                &target_device,
                                &config,
                                cmd_rx_clone.clone(),
                                thread_shared_state.clone(),
                            )?;

                            active_stream = Some(stream);
                            current_device_name = Some(dev_name.clone());

                            Ok(AudioDeviceItem {
                                name: dev_name,
                                is_default,
                                sample_rate,
                                channels,
                            })
                        })();

                        let _ = reply.send(res);
                    }
                    HostCommand::GetDevice { reply } => {
                        let _ = reply.send(current_device_name.clone());
                    }
                }
            }
        });

        let engine = AudioEngine {
            host_sender,
        };

        (engine, command_bus, shared_state)
    }

    fn build_stream(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        command_receiver: crossbeam_channel::Receiver<Command>,
        shared_state: Arc<SharedEngineState>,
    ) -> Result<cpal::Stream, String> {
        let device_sample_rate = config.sample_rate().0 as f64;
        shared_state.device_sample_rate.store(config.sample_rate().0 as usize, Ordering::SeqCst);

        let output_channels = config.channels() as usize;

            // Maximum supported channel count and buffer frames for real-time safety
            const MAX_CHANNELS: usize = 8;
            const MAX_BUFFER_FRAMES: usize = 16384;
            const MAX_BACKGROUND_VOICES: usize = 4;

            // Local state for active track voice
            let mut active_audio: Option<Arc<DecodedAudio>> = None;
            let mut playback_frame: usize = 0;
            let mut is_playing = false;
            let mut current_speed: f32 = 1.0;
            let mut current_pitch: f32 = 0.0;
            let mut stretch = signalsmith_stretch_rs::SignalsmithStretch::new(output_channels.max(2).min(MAX_CHANNELS), device_sample_rate as f32);
            stretch.set_transpose_semitones(current_pitch);
            let mut stretch_channels: usize = output_channels.max(2).min(MAX_CHANNELS);

            // Pre-allocated scratch buffers for active track
            let mut in_channel_scratch: Vec<Vec<f32>> = vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS];
            let mut out_channel_scratch: Vec<Vec<f32>> = vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS];

            // Background voice pool and block scratch buffer
            let mut background_voices: [Option<BackgroundVoice>; MAX_BACKGROUND_VOICES] = std::array::from_fn(|_| None);
            let mut bg_voice_block_scratch: Vec<f32> = vec![0.0f32; MAX_BUFFER_FRAMES * MAX_CHANNELS];

            // Pre-allocate biquad filter pool (up to 16 cascade bands)
            let mut biquads_pool: Vec<crate::dsp::Biquad> = (0..crate::command::MAX_EQ_BANDS).map(|_| crate::dsp::Biquad::new(output_channels)).collect();
            let mut active_biquad_count = 0;
            let mut eq_active = false;
            let mut dual_compressor = crate::dsp::DualCompressor::new(device_sample_rate);
            let mut active_regions: [crate::command::EngineRegion; crate::command::MAX_ENGINE_REGIONS] = [crate::command::EngineRegion::default(); crate::command::MAX_ENGINE_REGIONS];
            let mut active_region_count: usize = 0;
            let mut envelope_nodes = [crate::command::EnvelopeNode::default(); crate::command::MAX_ENVELOPE_NODES];
            let mut envelope_nodes_count: usize = 0;

            let shared_is_playing = shared_state.is_playing.clone();
            let shared_current_frame = shared_state.current_frame.clone();
            let shared_total_frames = shared_state.total_frames.clone();
            let shared_sample_rate = shared_state.sample_rate.clone();
            let shared_volume_raw = shared_state.volume_raw.clone();
            let shared_in_peak_l = shared_state.in_peak_db_l.clone();
            let shared_in_peak_r = shared_state.in_peak_db_r.clone();
            let shared_out_peak_l = shared_state.out_peak_db_l.clone();
            let shared_out_peak_r = shared_state.out_peak_db_r.clone();
            let shared_gr_stage1 = shared_state.gr_stage1_db.clone();
            let shared_gr_stage2 = shared_state.gr_stage2_db.clone();
            let shared_background_tracks = shared_state.background_tracks_count.clone();

            let err_fn = |err| log::error!("An error occurred on the audio stream: {}", err);

            match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    let s = device.build_output_stream(
                        &config.clone().into(),
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            // Update volume from main thread state if changed there
                            let vol_raw = shared_volume_raw.load(Ordering::SeqCst);
                            let volume = vol_raw as f32 / 1000.0;

                            // 1. Process pending commands with stack-allocated parameter coalescing
                            let mut pending_pitch: Option<f32> = None;
                            let mut pending_tempo: Option<f32> = None;
                            let mut pending_eq_bands: Option<([crate::command::EqBand; crate::command::MAX_EQ_BANDS], usize)> = None;
                            let mut pending_dual_comp: Option<(crate::dsp::CompStageParams, crate::dsp::CompStageParams, crate::dsp::CompRouting, f32)> = None;
                            let mut pending_regions: Option<([crate::command::EngineRegion; crate::command::MAX_ENGINE_REGIONS], usize)> = None;
                            let mut pending_envelope: Option<([crate::command::EnvelopeNode; crate::command::MAX_ENVELOPE_NODES], usize)> = None;

                            while let Ok(cmd) = command_receiver.try_recv() {
                                match cmd {
                                    Command::Play => {
                                        is_playing = true;
                                        shared_is_playing.store(true, Ordering::SeqCst);
                                    }
                                    Command::Pause => {
                                        // Pause freezes both active track and all background tracks
                                        is_playing = false;
                                        shared_is_playing.store(false, Ordering::SeqCst);
                                    }
                                    Command::Stop => {
                                        // Stop rewinds active track and completely terminates all background tracks
                                        is_playing = false;
                                        playback_frame = 0;
                                        shared_is_playing.store(false, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        stretch.reset();
                                        for b in &mut biquads_pool {
                                            b.reset();
                                        }
                                        dual_compressor.reset();

                                        for slot in &mut background_voices {
                                            *slot = None;
                                        }
                                        shared_background_tracks.store(0, Ordering::SeqCst);
                                    }
                                    Command::StopBackgroundTracks => {
                                        for slot in &mut background_voices {
                                            *slot = None;
                                        }
                                        shared_background_tracks.store(0, Ordering::SeqCst);
                                    }
                                    Command::Seek(duration) => {
                                        if let Some(ref audio) = active_audio {
                                            let frame_rate = audio.sample_rate as f64;
                                            let target_frame = (duration.as_secs_f64() * frame_rate) as usize;
                                            let total = audio.channel_samples[0].len();
                                            playback_frame = std::cmp::min(target_frame, total);
                                            shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                            stretch.reset();
                                        }
                                    }
                                    Command::SetVolume(vol) => {
                                        shared_volume_raw.store((vol.max(0.0) * 1000.0) as usize, Ordering::SeqCst);
                                    }
                                    Command::SetPitch(pitch) => {
                                        pending_pitch = Some(pitch);
                                    }
                                    Command::SetTempo(speed) => {
                                        pending_tempo = Some(speed);
                                    }
                                    Command::SetEq { bass_db, mid_db, treble_db } => {
                                        let mut bands = [crate::command::EqBand::default(); crate::command::MAX_EQ_BANDS];
                                        let mut count = 0;
                                        if bass_db.abs() > 0.001 {
                                             bands[count] = crate::command::EqBand {
                                                filter_type: crate::dsp::FilterType::LowShelf,
                                                freq: 100.0,
                                                gain_db: bass_db as f64,
                                                q: 0.707,
                                                enabled: true,
                                            };
                                            count += 1;
                                        }
                                        if mid_db.abs() > 0.001 {
                                            bands[count] = crate::command::EqBand {
                                                filter_type: crate::dsp::FilterType::Peaking,
                                                freq: 1000.0,
                                                gain_db: mid_db as f64,
                                                q: 1.0,
                                                enabled: true,
                                            };
                                            count += 1;
                                        }
                                        if treble_db.abs() > 0.001 {
                                            bands[count] = crate::command::EqBand {
                                                filter_type: crate::dsp::FilterType::HighShelf,
                                                freq: 8000.0,
                                                gain_db: treble_db as f64,
                                                q: 0.707,
                                                enabled: true,
                                            };
                                            count += 1;
                                        }
                                        pending_eq_bands = Some((bands, count));
                                    }
                                    Command::SetEqBands(bands, count) => {
                                        pending_eq_bands = Some((bands, count));
                                    }
                                    Command::SetCompressor { threshold_db, ratio, makeup_db, attack_ms, release_ms } => {
                                        let stage1 = crate::dsp::CompStageParams {
                                            enabled: true,
                                            comp_type: crate::dsp::CompType::Vintage,
                                            threshold_db,
                                            ratio,
                                            knee_db: 3.0,
                                            attack_ms,
                                            release_ms,
                                            makeup_db,
                                        };
                                        let stage2 = crate::dsp::CompStageParams {
                                            enabled: false,
                                            ..Default::default()
                                        };
                                        pending_dual_comp = Some((stage1, stage2, crate::dsp::CompRouting::Series, 0.5));
                                    }
                                    Command::SetDualCompressor { stage1, stage2, routing, parallel_blend } => {
                                        pending_dual_comp = Some((stage1, stage2, routing, parallel_blend));
                                    }
                                    Command::SetRegions(regs, count) => {
                                        pending_regions = Some((regs, count));
                                    }
                                    Command::SetVolumeEnvelope(nodes, count) => {
                                        pending_envelope = Some((nodes, count));
                                    }
                                    Command::LoadAudio(audio) => {
                                        let audio_to_load = audio;
                                        if (audio_to_load.sample_rate as f64 - device_sample_rate).abs() > 0.5 && audio_to_load.sample_rate > 0 {
                                            log::warn!(
                                                "Audio sample rate ({} Hz) differs from device sample rate ({} Hz) in audio callback. Audio must be pre-resampled off-thread.",
                                                audio_to_load.sample_rate,
                                                device_sample_rate
                                            );
                                        }

                                        // If a track was currently playing, transfer it to the background voices pool!
                                        if is_playing {
                                            if let Some(old_audio) = active_audio.take() {
                                                let old_len = old_audio.channel_samples[0].len();
                                                if playback_frame < old_len {
                                                    let old_stretch = std::mem::replace(
                                                        &mut stretch,
                                                        signalsmith_stretch_rs::SignalsmithStretch::new(output_channels.max(2).min(MAX_CHANNELS), device_sample_rate as f32)
                                                    );
                                                    let old_biquads = std::mem::replace(
                                                        &mut biquads_pool,
                                                        (0..crate::command::MAX_EQ_BANDS).map(|_| crate::dsp::Biquad::new(output_channels)).collect()
                                                    );
                                                    let old_comp = std::mem::replace(
                                                        &mut dual_compressor,
                                                        crate::dsp::DualCompressor::new(device_sample_rate)
                                                    );
                                                    let old_in_scratch = std::mem::replace(
                                                        &mut in_channel_scratch,
                                                        vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS]
                                                    );
                                                    let old_out_scratch = std::mem::replace(
                                                        &mut out_channel_scratch,
                                                        vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS]
                                                    );

                                                    let bg_voice = BackgroundVoice {
                                                        audio: old_audio,
                                                        playback_frame,
                                                        speed: current_speed,
                                                        pitch: current_pitch,
                                                        stretch: old_stretch,
                                                        stretch_channels,
                                                        biquads_pool: old_biquads,
                                                        active_biquad_count,
                                                        eq_active,
                                                        dual_compressor: old_comp,
                                                        volume,
                                                        envelope_nodes,
                                                        envelope_nodes_count,
                                                        active_regions,
                                                        active_region_count,
                                                        in_scratch: old_in_scratch,
                                                        out_scratch: old_out_scratch,
                                                    };

                                                    // Reset active track controls
                                                    stretch_channels = output_channels.max(2).min(MAX_CHANNELS);
                                                    stretch.set_transpose_semitones(0.0);
                                                    current_pitch = 0.0;
                                                    current_speed = 1.0;
                                                    active_biquad_count = 0;
                                                    eq_active = false;
                                                    envelope_nodes_count = 0;
                                                    active_region_count = 0;

                                                    if let Some(slot) = background_voices.iter_mut().find(|s| s.is_none()) {
                                                        *slot = Some(bg_voice);
                                                    } else {
                                                        // Pool full: displace oldest voice
                                                        for i in 0..MAX_BACKGROUND_VOICES - 1 {
                                                            background_voices[i] = background_voices[i + 1].take();
                                                        }
                                                        background_voices[MAX_BACKGROUND_VOICES - 1] = Some(bg_voice);
                                                    }

                                                    let active_bg = background_voices.iter().filter(|s| s.is_some()).count();
                                                    shared_background_tracks.store(active_bg, Ordering::Relaxed);
                                                }
                                            }
                                        }

                                        let total = audio_to_load.channel_samples[0].len();
                                        let rate = audio_to_load.sample_rate;
                                        shared_total_frames.store(total, Ordering::SeqCst);
                                        shared_sample_rate.store(rate as usize, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        
                                        let ch = audio_to_load.channels.max(1).min(MAX_CHANNELS);
                                        stretch_channels = ch;
                                        stretch.preset_default(stretch_channels, device_sample_rate as f32);
                                        stretch.reset();
                                        stretch.set_transpose_semitones(current_pitch);

                                        for b in &mut biquads_pool {
                                            b.reset();
                                        }
                                        dual_compressor.reset();

                                        active_audio = Some(audio_to_load);
                                        playback_frame = 0;
                                    }
                                }
                            }

                            // Apply coalesced parameter updates exactly once per buffer block
                            if let Some(pitch) = pending_pitch {
                                current_pitch = pitch;
                                stretch.set_transpose_semitones(current_pitch);
                            }
                            if let Some(speed) = pending_tempo {
                                current_speed = speed.clamp(0.25, 4.0);
                            }
                            if let Some((bands, count)) = pending_eq_bands {
                                let mut biquad_idx = 0;
                                for band in bands.iter().take(count) {
                                    if band.enabled && (band.gain_db.abs() > 0.01 || matches!(band.filter_type, crate::dsp::FilterType::LowPass | crate::dsp::FilterType::HighPass | crate::dsp::FilterType::Notch)) {
                                        if biquad_idx < biquads_pool.len() {
                                            biquads_pool[biquad_idx].set_params(band.filter_type, device_sample_rate, band.freq, band.gain_db, band.q);
                                            biquad_idx += 1;
                                        }
                                    }
                                }
                                active_biquad_count = biquad_idx;
                                eq_active = active_biquad_count > 0;
                            }
                            if let Some((stage1, stage2, routing, parallel_blend)) = pending_dual_comp {
                                dual_compressor.stage1.set_params(device_sample_rate, stage1);
                                dual_compressor.stage2.set_params(device_sample_rate, stage2);
                                dual_compressor.routing = routing;
                                dual_compressor.parallel_blend = parallel_blend;
                            }
                            if let Some((regs, count)) = pending_regions {
                                active_regions = regs;
                                active_region_count = count;
                            }
                            if let Some((nodes, count)) = pending_envelope {
                                envelope_nodes = nodes;
                                envelope_nodes_count = count;
                            }

                            // 2. Render samples
                            let num_out_frames = data.len() / output_channels;

                            if !is_playing {
                                for sample in data.iter_mut() {
                                    *sample = 0.0;
                                }
                                shared_in_peak_l.store(-6000, Ordering::Relaxed);
                                shared_in_peak_r.store(-6000, Ordering::Relaxed);
                                shared_out_peak_l.store(-6000, Ordering::Relaxed);
                                shared_out_peak_r.store(-6000, Ordering::Relaxed);
                                shared_gr_stage1.store(0, Ordering::Relaxed);
                                shared_gr_stage2.store(0, Ordering::Relaxed);
                            } else {
                                let mut has_active = false;
                                let mut active_finished = false;

                                if let Some(ref audio) = active_audio {
                                    has_active = true;
                                    active_finished = render_single_voice(
                                        audio,
                                        &mut playback_frame,
                                        current_speed,
                                        current_pitch,
                                        &mut stretch,
                                        stretch_channels,
                                        &mut biquads_pool,
                                        active_biquad_count,
                                        eq_active,
                                        &mut dual_compressor,
                                        volume,
                                        &envelope_nodes[..envelope_nodes_count],
                                        &active_regions[..active_region_count],
                                        device_sample_rate,
                                        output_channels,
                                        num_out_frames,
                                        &mut in_channel_scratch,
                                        &mut out_channel_scratch,
                                        data,
                                    );

                                    if active_finished {
                                        shared_is_playing.store(false, Ordering::SeqCst);
                                    }
                                    shared_current_frame.store(playback_frame, Ordering::SeqCst);

                                    // Measure compressor gain reduction on active track
                                    let gr1 = dual_compressor.stage1.last_gr_db.abs();
                                    let gr2 = dual_compressor.stage2.last_gr_db.abs();
                                    shared_gr_stage1.store((gr1 * 100.0) as i32, Ordering::Relaxed);
                                    shared_gr_stage2.store((gr2 * 100.0) as i32, Ordering::Relaxed);
                                } else {
                                    for sample in data.iter_mut() {
                                        *sample = 0.0;
                                    }
                                    shared_gr_stage1.store(0, Ordering::Relaxed);
                                    shared_gr_stage2.store(0, Ordering::Relaxed);
                                }

                                // Render background voices into bg_voice_block_scratch and sum into data
                                let mut active_bg_count = 0;
                                let total_block_samples = num_out_frames * output_channels;
                                for slot in &mut background_voices {
                                    if let Some(ref mut voice) = slot {
                                        let voice_finished = render_single_voice(
                                            &voice.audio,
                                            &mut voice.playback_frame,
                                            voice.speed,
                                            voice.pitch,
                                            &mut voice.stretch,
                                            voice.stretch_channels,
                                            &mut voice.biquads_pool,
                                            voice.active_biquad_count,
                                            voice.eq_active,
                                            &mut voice.dual_compressor,
                                            voice.volume,
                                            &voice.envelope_nodes[..voice.envelope_nodes_count],
                                            &voice.active_regions[..voice.active_region_count],
                                            device_sample_rate,
                                            output_channels,
                                            num_out_frames,
                                            &mut voice.in_scratch,
                                            &mut voice.out_scratch,
                                            &mut bg_voice_block_scratch[..total_block_samples],
                                        );

                                        for i in 0..total_block_samples {
                                            data[i] += bg_voice_block_scratch[i];
                                        }

                                        if voice_finished {
                                            *slot = None;
                                        } else {
                                            active_bg_count += 1;
                                        }
                                    }
                                }
                                shared_background_tracks.store(active_bg_count, Ordering::Relaxed);

                                // If both active track finished (or none) and all background tracks finished:
                                if (!has_active || active_finished) && active_bg_count == 0 {
                                    is_playing = false;
                                    shared_is_playing.store(false, Ordering::SeqCst);
                                }

                                // Measure output levels on final mixed audio
                                let mut max_in_l: f32 = 0.0;
                                let mut max_in_r: f32 = 0.0;
                                let mut max_out_l: f32 = 0.0;
                                let mut max_out_r: f32 = 0.0;
                                for frame_idx in 0..num_out_frames {
                                    let l_idx = frame_idx * output_channels;
                                    let r_idx = if output_channels > 1 { l_idx + 1 } else { l_idx };
                                    max_in_l = max_in_l.max(data[l_idx].abs());
                                    max_in_r = max_in_r.max(data[r_idx].abs());
                                    max_out_l = max_out_l.max(data[l_idx].abs());
                                    max_out_r = max_out_r.max(data[r_idx].abs());
                                }
                                let in_l_db = if max_in_l > 1e-5 { (20.0 * max_in_l.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                let in_r_db = if max_in_r > 1e-5 { (20.0 * max_in_r.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                shared_in_peak_l.store((in_l_db * 100.0) as i32, Ordering::Relaxed);
                                shared_in_peak_r.store((in_r_db * 100.0) as i32, Ordering::Relaxed);
                                let out_l_db = if max_out_l > 1e-5 { (20.0 * max_out_l.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                let out_r_db = if max_out_r > 1e-5 { (20.0 * max_out_r.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                shared_out_peak_l.store((out_l_db * 100.0) as i32, Ordering::Relaxed);
                                shared_out_peak_r.store((out_r_db * 100.0) as i32, Ordering::Relaxed);
                            }

                            shared_current_frame.store(playback_frame, Ordering::SeqCst);
                        },
                        err_fn,
                        None
                    );
                    let stream = s.map_err(|e| format!("Failed to build CPAL output stream: {}", e))?;
                    stream.play().map_err(|e| format!("Failed to start CPAL stream: {}", e))?;
                    Ok(stream)
                }
                sample_fmt => {
                    Err(format!("Unsupported output sample format: {:?}", sample_fmt))
                }
            }
        }

    pub fn set_output_device(&self, device_name: Option<&str>) -> Result<AudioDeviceItem, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.host_sender.send(HostCommand::SetDevice {
            name: device_name.map(|s| s.to_string()),
            reply: reply_tx,
        }).map_err(|e| format!("Audio host thread disconnected: {}", e))?;

        reply_rx.recv().map_err(|e| format!("Failed to receive device switch reply: {}", e))?
    }

    pub fn current_device(&self) -> Option<String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        if self.host_sender.send(HostCommand::GetDevice { reply: reply_tx }).is_ok() {
            reply_rx.recv().ok().flatten()
        } else {
            None
        }
    }

    pub fn start(&self) -> Result<(), String> {
        self.set_output_device(None).map(|_| ())
    }
}
