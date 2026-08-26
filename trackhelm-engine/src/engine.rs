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
    pub volume_raw: Arc<AtomicUsize>, // Volume scaled by 1000 (e.g. 1.0 -> 1000)
    pub in_peak_db_l: Arc<AtomicI32>,  // (db * 100.0) as i32, default -6000 (-60.0 dB)
    pub in_peak_db_r: Arc<AtomicI32>,
    pub out_peak_db_l: Arc<AtomicI32>,
    pub out_peak_db_r: Arc<AtomicI32>,
    pub gr_stage1_db: Arc<AtomicI32>,  // (abs_gr_db * 100.0) as i32, default 0
    pub gr_stage2_db: Arc<AtomicI32>,
}

pub struct AudioEngine {
    command_receiver: crossbeam_channel::Receiver<Command>,
    shared_state: Arc<SharedEngineState>,
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
            volume_raw: Arc::new(AtomicUsize::new(1000)), // default 1.0 volume
            in_peak_db_l: Arc::new(AtomicI32::new(-6000)),
            in_peak_db_r: Arc::new(AtomicI32::new(-6000)),
            out_peak_db_l: Arc::new(AtomicI32::new(-6000)),
            out_peak_db_r: Arc::new(AtomicI32::new(-6000)),
            gr_stage1_db: Arc::new(AtomicI32::new(0)),
            gr_stage2_db: Arc::new(AtomicI32::new(0)),
        });

        let engine = AudioEngine {
            command_receiver: receiver,
            shared_state: shared_state.clone(),
        };

        (engine, command_bus, shared_state)
    }

    pub fn start(&mut self) -> Result<(), String> {
        let command_receiver = self.command_receiver.clone();
        let shared_state = self.shared_state.clone();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_output_device() {
                Some(d) => d,
                None => {
                    log::error!("No default audio output device found");
                    return;
                }
            };

            let config = match device.default_output_config() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to get default output config: {}", e);
                    return;
                }
            };

            log::info!("Audio device initialized: {}", device.name().unwrap_or_default());
            log::info!("Default output config: {:?}", config);

            let output_channels = config.channels() as usize;

            // Maximum supported channel count and buffer frames for real-time safety
            const MAX_CHANNELS: usize = 8;
            const MAX_BUFFER_FRAMES: usize = 16384;

            // Local state for the audio thread
            let mut active_audio: Option<Arc<DecodedAudio>> = None;
            let mut playback_frame: usize = 0;
            let mut is_playing = false;
            let mut current_speed: f32 = 1.0;
            let mut current_pitch: f32 = 0.0;
            let stretch = signalsmith_stretch_rs::SignalsmithStretch::new(MAX_CHANNELS, config.sample_rate().0 as f32);
            stretch.set_transpose_semitones(current_pitch);
            let mut stretch_channels: usize = 2;

            // Pre-allocated scratch buffers (zero heap allocations in audio loop)
            let mut in_channel_scratch: Vec<Vec<f32>> = vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS];
            let mut out_channel_scratch: Vec<Vec<f32>> = vec![vec![0.0f32; MAX_BUFFER_FRAMES]; MAX_CHANNELS];

            let mut current_sample_rate = config.sample_rate().0 as f64;
            // Pre-allocate biquad filter pool (up to 16 cascade bands)
            let mut biquads_pool: Vec<crate::dsp::Biquad> = (0..crate::command::MAX_EQ_BANDS).map(|_| crate::dsp::Biquad::new(output_channels)).collect();
            let mut active_biquad_count = 0;
            let mut eq_active = false;
            let mut dual_compressor = crate::dsp::DualCompressor::new(current_sample_rate);
            let mut active_regions: [crate::command::EngineRegion; crate::command::MAX_ENGINE_REGIONS] = [crate::command::EngineRegion::default(); crate::command::MAX_ENGINE_REGIONS];
            let mut active_region_count: usize = 0;

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

            let err_fn = |err| log::error!("An error occurred on the audio stream: {}", err);

            // Keep the stream alive inside this thread by holding its handle
            let _stream = match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    let s = device.build_output_stream(
                        &config.into(),
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

                            while let Ok(cmd) = command_receiver.try_recv() {
                                match cmd {
                                    Command::Play => {
                                        is_playing = true;
                                        shared_is_playing.store(true, Ordering::SeqCst);
                                    }
                                    Command::Pause => {
                                        is_playing = false;
                                        shared_is_playing.store(false, Ordering::SeqCst);
                                    }
                                    Command::Stop => {
                                        is_playing = false;
                                        playback_frame = 0;
                                        shared_is_playing.store(false, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        stretch.reset();
                                        for b in &mut biquads_pool {
                                            b.reset();
                                        }
                                        dual_compressor.reset();
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
                                                q: 0.707,
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
                                    Command::LoadAudio(audio) => {
                                        let total = audio.channel_samples[0].len();
                                        let rate = audio.sample_rate;
                                        current_sample_rate = rate as f64;
                                        shared_total_frames.store(total, Ordering::SeqCst);
                                        shared_sample_rate.store(rate as usize, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        
                                        let ch = audio.channels.max(1).min(MAX_CHANNELS);
                                        stretch_channels = ch;
                                        stretch.reset();
                                        stretch.set_transpose_semitones(current_pitch);

                                        for b in &mut biquads_pool {
                                            b.reset();
                                        }
                                        dual_compressor.reset();

                                        active_audio = Some(audio);
                                        playback_frame = 0;
                                    }
                                }
                            }

                            // Apply coalesced parameter updates exactly once per buffer block (zero heap allocations)
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
                                            biquads_pool[biquad_idx].set_params(band.filter_type, current_sample_rate, band.freq, band.gain_db, band.q);
                                            biquad_idx += 1;
                                        }
                                    }
                                }
                                active_biquad_count = biquad_idx;
                                eq_active = active_biquad_count > 0;
                            }
                            if let Some((stage1, stage2, routing, parallel_blend)) = pending_dual_comp {
                                dual_compressor.stage1.set_params(current_sample_rate, stage1);
                                dual_compressor.stage2.set_params(current_sample_rate, stage2);
                                dual_compressor.routing = routing;
                                dual_compressor.parallel_blend = parallel_blend;
                            }
                            if let Some((regs, count)) = pending_regions {
                                active_regions = regs;
                                active_region_count = count;
                            }

                            // 2. Render samples
                            let num_out_frames = data.len() / output_channels;

                            if !is_playing || active_audio.is_none() {
                                for sample in data.iter_mut() {
                                    *sample = 0.0;
                                }
                            } else if let Some(ref audio) = active_audio {
                                let audio_len = audio.channel_samples[0].len();
                                let audio_channels = audio.channels;
                                let frame_rate = audio.sample_rate as f64;

                                // Region handling: Check for Cut skip and Loop wrap
                                let current_sec = playback_frame as f64 / frame_rate;
                                for reg in &active_regions[..active_region_count] {
                                    if reg.is_cut && current_sec >= reg.start_seconds && current_sec < reg.end_seconds {
                                        playback_frame = (reg.end_seconds * frame_rate) as usize;
                                        shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                        stretch.reset();
                                        break;
                                    } else if reg.is_loop && current_sec >= reg.end_seconds {
                                        playback_frame = (reg.start_seconds * frame_rate) as usize;
                                        shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                        stretch.reset();
                                        break;
                                    }
                                }

                                if playback_frame >= audio_len {
                                    is_playing = false;
                                    shared_is_playing.store(false, Ordering::SeqCst);
                                    for sample in data.iter_mut() {
                                        *sample = 0.0;
                                    }
                                } else {
                                    let is_passthrough = (current_speed - 1.0).abs() < 0.001 && current_pitch.abs() < 0.001;

                                    if is_passthrough {
                                        // Direct playback without stretch
                                        for frame_idx in 0..num_out_frames {
                                            if playback_frame < audio_len {
                                                for out_c in 0..output_channels {
                                                    let in_c = out_c % audio_channels;
                                                    data[frame_idx * output_channels + out_c] = audio.channel_samples[in_c][playback_frame] * volume;
                                                }
                                                playback_frame += 1;
                                            } else {
                                                for out_c in 0..output_channels {
                                                    data[frame_idx * output_channels + out_c] = 0.0;
                                                }
                                                is_playing = false;
                                                shared_is_playing.store(false, Ordering::SeqCst);
                                            }
                                        }
                                    } else {
                                        // Stretch processing using pre-allocated scratch buffers
                                        let num_in_frames = ((num_out_frames as f32) * current_speed).round() as usize;
                                        let safe_in_frames = std::cmp::min(num_in_frames, MAX_BUFFER_FRAMES);
                                        let safe_out_frames = std::cmp::min(num_out_frames, MAX_BUFFER_FRAMES);

                                        for i in 0..safe_in_frames {
                                            let curr_f = playback_frame + i;
                                            if curr_f < audio_len {
                                                for ch in 0..stretch_channels {
                                                    in_channel_scratch[ch][i] = audio.channel_samples[ch % audio_channels][curr_f];
                                                }
                                            } else {
                                                for ch in 0..stretch_channels {
                                                    in_channel_scratch[ch][i] = 0.0;
                                                }
                                            }
                                        }

                                        // Form zero-allocation channel slices for Signalsmith processing
                                        let mut in_slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                                        for ch in 0..stretch_channels {
                                            in_slices[ch] = &in_channel_scratch[ch][..safe_in_frames];
                                        }

                                        let mut out_slices: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|ch| {
                                            if ch < stretch_channels {
                                                unsafe {
                                                    std::slice::from_raw_parts_mut(out_channel_scratch[ch].as_mut_ptr(), safe_out_frames)
                                                }
                                            } else {
                                                &mut [][..]
                                            }
                                        });

                                        stretch.process(&in_slices[..stretch_channels], &mut out_slices[..stretch_channels]);

                                        for frame_idx in 0..num_out_frames {
                                            for out_c in 0..output_channels {
                                                let in_c = out_c % stretch_channels;
                                                data[frame_idx * output_channels + out_c] = out_channel_scratch[in_c][frame_idx] * volume;
                                            }
                                        }

                                        playback_frame += num_in_frames;
                                        if playback_frame >= audio_len {
                                            is_playing = false;
                                            shared_is_playing.store(false, Ordering::SeqCst);
                                        }
                                    }

                                    // Measure input levels (before EQ & compressor)
                                    let mut max_in_l: f32 = 0.0;
                                    let mut max_in_r: f32 = 0.0;
                                    for frame_idx in 0..num_out_frames {
                                        let l_idx = frame_idx * output_channels;
                                        let r_idx = if output_channels > 1 { l_idx + 1 } else { l_idx };
                                        max_in_l = max_in_l.max(data[l_idx].abs());
                                        max_in_r = max_in_r.max(data[r_idx].abs());
                                    }
                                    let in_l_db = if max_in_l > 1e-5 { (20.0 * max_in_l.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                    let in_r_db = if max_in_r > 1e-5 { (20.0 * max_in_r.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                    shared_in_peak_l.store((in_l_db * 100.0) as i32, Ordering::Relaxed);
                                    shared_in_peak_r.store((in_r_db * 100.0) as i32, Ordering::Relaxed);

                                    // 3. Apply High-Quality Biquad EQ Filters (in-place)
                                    if eq_active && active_biquad_count > 0 {
                                        for frame_idx in 0..num_out_frames {
                                            for ch in 0..output_channels {
                                                let idx = frame_idx * output_channels + ch;
                                                let mut s = data[idx];
                                                for b in &mut biquads_pool[..active_biquad_count] {
                                                    s = b.process_sample(ch, s);
                                                }
                                                data[idx] = s;
                                            }
                                        }
                                    }

                                    // 4. Apply Dual-Stage Dynamic Compressor
                                    if !dual_compressor.is_bypassed() {
                                        for frame_idx in 0..num_out_frames {
                                            let left_idx = frame_idx * output_channels;
                                            let right_idx = if output_channels > 1 { left_idx + 1 } else { left_idx };
                                            let (l, r) = dual_compressor.process_stereo_frame(data[left_idx], data[right_idx]);
                                            data[left_idx] = l;
                                            if output_channels > 1 {
                                                data[right_idx] = r;
                                            }
                                        }
                                    }

                                    // Measure gain reduction and output levels
                                    let gr1 = dual_compressor.stage1.last_gr_db.abs();
                                    let gr2 = dual_compressor.stage2.last_gr_db.abs();
                                    shared_gr_stage1.store((gr1 * 100.0) as i32, Ordering::Relaxed);
                                    shared_gr_stage2.store((gr2 * 100.0) as i32, Ordering::Relaxed);

                                    let mut max_out_l: f32 = 0.0;
                                    let mut max_out_r: f32 = 0.0;
                                    for frame_idx in 0..num_out_frames {
                                        let l_idx = frame_idx * output_channels;
                                        let r_idx = if output_channels > 1 { l_idx + 1 } else { l_idx };
                                        max_out_l = max_out_l.max(data[l_idx].abs());
                                        max_out_r = max_out_r.max(data[r_idx].abs());
                                    }
                                    let out_l_db = if max_out_l > 1e-5 { (20.0 * max_out_l.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                    let out_r_db = if max_out_r > 1e-5 { (20.0 * max_out_r.log10()).clamp(-60.0, 6.0) } else { -60.0 };
                                    shared_out_peak_l.store((out_l_db * 100.0) as i32, Ordering::Relaxed);
                                    shared_out_peak_r.store((out_r_db * 100.0) as i32, Ordering::Relaxed);
                                }
                            } else {
                                shared_in_peak_l.store(-6000, Ordering::Relaxed);
                                shared_in_peak_r.store(-6000, Ordering::Relaxed);
                                shared_out_peak_l.store(-6000, Ordering::Relaxed);
                                shared_out_peak_r.store(-6000, Ordering::Relaxed);
                                shared_gr_stage1.store(0, Ordering::Relaxed);
                                shared_gr_stage2.store(0, Ordering::Relaxed);
                            }

                            shared_current_frame.store(playback_frame, Ordering::SeqCst);
                        },
                        err_fn,
                        None
                    );
                    match s {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::error!("Failed to start CPAL stream: {}", e);
                                return;
                            }
                            stream
                        }
                        Err(e) => {
                            log::error!("Failed to build CPAL output stream: {}", e);
                            return;
                        }
                    }
                }
                sample_fmt => {
                    log::error!("Unsupported output sample format: {:?}", sample_fmt);
                    return;
                }
            };

            // Loop to keep stream alive in background thread
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        });

        Ok(())
    }
}
