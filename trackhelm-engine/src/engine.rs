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
                                    }
                                    Command::Seek(duration) => {
                                        if let Some(ref audio) = active_audio {
                                            let frame_rate = audio.sample_rate as f64;
                                            let target_frame = (duration.as_secs_f64() * frame_rate) as usize;
                                            let total = audio.channel_samples[0].len();
                                            playback_frame = std::cmp::min(target_frame, total);
                                            shared_current_frame.store(playback_frame, Ordering::SeqCst);
                                        }
                                    }
                                    Command::SetVolume(vol) => {
                                        shared_volume_raw.store((vol.max(0.0) * 1000.0) as usize, Ordering::SeqCst);
                                    }
                                    Command::LoadAudio(audio) => {
                                        let total = audio.channel_samples[0].len();
                                        let rate = audio.sample_rate;
                                        shared_total_frames.store(total, Ordering::SeqCst);
                                        shared_sample_rate.store(rate as usize, Ordering::SeqCst);
                                        shared_current_frame.store(0, Ordering::SeqCst);
                                        active_audio = Some(audio);
                                        playback_frame = 0;
                                    }
                                    Command::SetPitch(_) | Command::SetTempo(_) => {
                                        // Handled in Milestone 2
                                    }
                                }
                            }

                            // 2. Render samples
                            let num_frames = data.len() / output_channels;
                            let mut samples_to_write = [0.0f32; 16]; // Real-time safe stack allocation
                            let channels_to_copy = std::cmp::min(output_channels, 16);

                            for frame_idx in 0..num_frames {
                                // Clear samples
                                for c in 0..channels_to_copy {
                                    samples_to_write[c] = 0.0;
                                }

                                if is_playing {
                                    if let Some(ref audio) = active_audio {
                                        let audio_len = audio.channel_samples[0].len();
                                        let audio_channels = audio.channels;

                                        if playback_frame < audio_len {
                                            for out_c in 0..channels_to_copy {
                                                let in_c = out_c % audio_channels;
                                                samples_to_write[out_c] = audio.channel_samples[in_c][playback_frame] * volume;
                                            }
                                            playback_frame += 1;
                                        } else {
                                            is_playing = false;
                                            shared_is_playing.store(false, Ordering::SeqCst);
                                        }
                                    }
                                }

                                // Write to the output buffer
                                for out_c in 0..output_channels {
                                    let write_val = if out_c < 16 { samples_to_write[out_c] } else { 0.0 };
                                    data[frame_idx * output_channels + out_c] = write_val;
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
