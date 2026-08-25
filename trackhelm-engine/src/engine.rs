use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

            // Local state for the audio thread
            let mut active_audio: Option<Arc<DecodedAudio>> = None;
            let mut playback_frame: usize = 0;
            let mut is_playing = false;
            let mut current_speed: f32 = 1.0;
            let mut current_pitch: f32 = 0.0;
            let mut stretch: Option<signalsmith_stretch_rs::SignalsmithStretch> = None;
            let mut stretch_channels: usize = 2;

            let mut stretch_in_buffers: Vec<Vec<f32>> = vec![Vec::new(); 2];
            let mut stretch_out_buffers: Vec<Vec<f32>> = vec![Vec::new(); 2];

            let mut current_sample_rate = config.sample_rate().0 as f64;
            let mut biquad_low = crate::dsp::Biquad::new(output_channels);
            let mut biquad_mid = crate::dsp::Biquad::new(output_channels);
            let mut biquad_high = crate::dsp::Biquad::new(output_channels);
            let mut eq_active = false;
            let mut compressor = crate::dsp::Compressor::new(current_sample_rate);
            let mut active_regions: Vec<crate::command::EngineRegion> = Vec::new();

            let shared_is_playing = shared_state.is_playing.clone();
            let shared_current_frame = shared_state.current_frame.clone();
            let shared_total_frames = shared_state.total_frames.clone();
            let shared_sample_rate = shared_state.sample_rate.clone();
            let shared_volume_raw = shared_state.volume_raw.clone();

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

                            // 1. Process pending commands
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
                                        if let Some(ref mut s) = stretch {
                                            s.reset();
                                        }
                                        biquad_low.reset();
                                        biquad_mid.reset();
                                        biquad_high.reset();
                                        compressor.reset();
                                    }
                                    Command::Seek(duration) => {
                                        if let Some(ref audio) = active_audio {
                                            let frame_rate = audio.sample_rate as f64;
                                            let target_frame = (duration.as_secs_f64() * frame_rate) as usize;
                                            let total = audio.channel_samples[0].len();
                                            playback_frame = std::cmp::min(target_frame, total);
                                            shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                            if let Some(ref mut s) = stretch {
                                                s.reset();
                                            }
                                        }
                                    }
                                    Command::SetVolume(vol) => {
                                        shared_volume_raw.store((vol.max(0.0) * 1000.0) as usize, Ordering::SeqCst);
                                    }
                                    Command::SetPitch(pitch) => {
                                        current_pitch = pitch;
                                        if let Some(ref s) = stretch {
                                            s.set_transpose_semitones(current_pitch);
                                        }
                                    }
                                    Command::SetTempo(speed) => {
                                        current_speed = speed.clamp(0.25, 4.0);
                                    }
                                    Command::SetEq { bass_db, mid_db, treble_db } => {
                                        eq_active = bass_db.abs() > 0.001 || mid_db.abs() > 0.001 || treble_db.abs() > 0.001;
                                        if eq_active {
                                            biquad_low.set_params(crate::dsp::FilterType::LowShelf, current_sample_rate, 100.0, bass_db as f64, 0.707);
                                            biquad_mid.set_params(crate::dsp::FilterType::Peaking, current_sample_rate, 1000.0, mid_db as f64, 0.707);
                                            biquad_high.set_params(crate::dsp::FilterType::HighShelf, current_sample_rate, 8000.0, treble_db as f64, 0.707);
                                        }
                                    }
                                    Command::SetCompressor { threshold_db, ratio, makeup_db, attack_ms, release_ms } => {
                                        compressor.set_params(current_sample_rate, threshold_db, ratio, makeup_db, attack_ms, release_ms);
                                    }
                                    Command::SetRegions(regs) => {
                                        active_regions = regs;
                                    }
                                    Command::LoadAudio(audio) => {
                                        let total = audio.channel_samples[0].len();
                                        let rate = audio.sample_rate;
                                        current_sample_rate = rate as f64;
                                        shared_total_frames.store(total, Ordering::SeqCst);
                                        shared_sample_rate.store(rate as usize, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        
                                        let ch = audio.channels.max(1);
                                        stretch_channels = ch;
                                        let s = signalsmith_stretch_rs::SignalsmithStretch::new(ch, rate as f32);
                                        s.set_transpose_semitones(current_pitch);
                                        stretch = Some(s);
                                        stretch_in_buffers = vec![Vec::new(); ch];
                                        stretch_out_buffers = vec![Vec::new(); ch];

                                        biquad_low = crate::dsp::Biquad::new(output_channels);
                                        biquad_mid = crate::dsp::Biquad::new(output_channels);
                                        biquad_high = crate::dsp::Biquad::new(output_channels);
                                        compressor = crate::dsp::Compressor::new(current_sample_rate);

                                        active_audio = Some(audio);
                                        playback_frame = 0;
                                    }
                                }
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
                                for reg in &active_regions {
                                    if reg.is_cut && current_sec >= reg.start_seconds && current_sec < reg.end_seconds {
                                        playback_frame = (reg.end_seconds * frame_rate) as usize;
                                        shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                        if let Some(ref mut s) = stretch {
                                            s.reset();
                                        }
                                        break;
                                    } else if reg.is_loop && current_sec >= reg.end_seconds {
                                        playback_frame = (reg.start_seconds * frame_rate) as usize;
                                        shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                        if let Some(ref mut s) = stretch {
                                            s.reset();
                                        }
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
                                    } else if let Some(ref stretch_inst) = stretch {
                                        // Stretch processing
                                        let num_in_frames = ((num_out_frames as f32) * current_speed).round() as usize;

                                        for ch in 0..stretch_channels {
                                            stretch_in_buffers[ch].resize(num_in_frames, 0.0);
                                            stretch_out_buffers[ch].resize(num_out_frames, 0.0);
                                        }

                                        for i in 0..num_in_frames {
                                            let curr_f = playback_frame + i;
                                            if curr_f < audio_len {
                                                for ch in 0..stretch_channels {
                                                    stretch_in_buffers[ch][i] = audio.channel_samples[ch % audio_channels][curr_f];
                                                }
                                            } else {
                                                for ch in 0..stretch_channels {
                                                    stretch_in_buffers[ch][i] = 0.0;
                                                }
                                            }
                                        }

                                        let in_slices: Vec<&[f32]> = stretch_in_buffers.iter().map(|v| v.as_slice()).collect();
                                        let mut out_slices: Vec<&mut [f32]> = stretch_out_buffers.iter_mut().map(|v| v.as_mut_slice()).collect();

                                        stretch_inst.process(&in_slices, &mut out_slices);

                                        for frame_idx in 0..num_out_frames {
                                            for out_c in 0..output_channels {
                                                let in_c = out_c % stretch_channels;
                                                data[frame_idx * output_channels + out_c] = stretch_out_buffers[in_c][frame_idx] * volume;
                                            }
                                        }

                                        playback_frame += num_in_frames;
                                        if playback_frame >= audio_len {
                                            is_playing = false;
                                            shared_is_playing.store(false, Ordering::SeqCst);
                                        }
                                    }

                                    // 3. Apply High-Quality Biquad EQ Filters
                                    if eq_active {
                                        for frame_idx in 0..num_out_frames {
                                            for ch in 0..output_channels {
                                                let idx = frame_idx * output_channels + ch;
                                                let mut s = data[idx];
                                                s = biquad_low.process_sample(ch, s);
                                                s = biquad_mid.process_sample(ch, s);
                                                s = biquad_high.process_sample(ch, s);
                                                data[idx] = s;
                                            }
                                        }
                                    }

                                    // 4. Apply Feedforward Soft-Knee Dynamic Compressor
                                    if !compressor.is_bypassed() {
                                        for frame_idx in 0..num_out_frames {
                                            let left_idx = frame_idx * output_channels;
                                            let right_idx = if output_channels > 1 { left_idx + 1 } else { left_idx };
                                            let (l, r) = compressor.process_stereo_frame(data[left_idx], data[right_idx]);
                                            data[left_idx] = l;
                                            if output_channels > 1 {
                                                data[right_idx] = r;
                                            }
                                        }
                                    }
                                }
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
