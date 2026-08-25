use std::path::Path;
use std::sync::Arc;
use crate::decoder::DecodedAudio;
use crate::command::{EqBand, EngineRegion};
use crate::dsp::{Biquad, DualCompressor, CompStageParams, CompRouting};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ExportBitDepth {
    Int16,
    Int24,
    Float32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportAudioConfig {
    pub output_path: String,
    pub bit_depth: ExportBitDepth,
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
    pub eq_bands: Vec<EqBand>,
    pub comp_stage1: CompStageParams,
    pub comp_stage2: CompStageParams,
    pub comp_routing: CompRouting,
    pub comp_parallel_blend: f32,
    pub regions: Vec<EngineRegion>,
}

pub fn render_audio_export(
    audio: &Arc<DecodedAudio>,
    config: &ExportAudioConfig,
) -> Result<usize, String> {
    let channels = audio.channels.max(1);
    let sample_rate = audio.sample_rate;
    let total_frames = audio.channel_samples[0].len();
    if total_frames == 0 {
        return Err("Audio track contains no samples to export".to_string());
    }

    let start_frame = config.range_start_seconds
        .map(|s| (s.max(0.0) * sample_rate as f64) as usize)
        .unwrap_or(0)
        .min(total_frames);

    let end_frame = config.range_end_seconds
        .map(|s| (s.max(0.0) * sample_rate as f64) as usize)
        .unwrap_or(total_frames)
        .min(total_frames);

    if start_frame >= end_frame {
        return Err("Invalid export range: start frame is greater than or equal to end frame".to_string());
    }

    // 1. Frame Extraction & Cut Region Splicing with Micro-Crossfades
    let mut spliced_channels: Vec<Vec<f32>> = vec![Vec::new(); channels];

    if config.bake_cuts && !config.regions.is_empty() {
        let mut cut_regions: Vec<&EngineRegion> = config.regions
            .iter()
            .filter(|r| r.is_cut)
            .collect();
        cut_regions.sort_by(|a, b| a.start_seconds.partial_cmp(&b.start_seconds).unwrap());

        let mut curr_frame = start_frame;
        while curr_frame < end_frame {
            let curr_sec = curr_frame as f64 / sample_rate as f64;
            
            // Check if curr_frame falls inside a cut region
            if let Some(cut) = cut_regions.iter().find(|r| curr_sec >= r.start_seconds && curr_sec < r.end_seconds) {
                let cut_end_frame = ((cut.end_seconds * sample_rate as f64) as usize).min(end_frame);
                let xfade_frames = ((cut.crossfade_ms.max(0.0) * sample_rate as f64 / 1000.0) as usize).min(1024);

                // Apply smooth equal-power crossfade at splice joint if frames are available
                if xfade_frames > 0 && spliced_channels[0].len() >= xfade_frames && cut_end_frame + xfade_frames <= total_frames {
                    let len_before = spliced_channels[0].len();
                    let fade_start_idx = len_before - xfade_frames;

                    for i in 0..xfade_frames {
                        let t = i as f32 / xfade_frames as f32;
                        let gain_out = (1.0 - t).max(0.0);
                        let gain_in = t;

                        for c in 0..channels {
                            let old_sample = spliced_channels[c][fade_start_idx + i];
                            let new_sample = audio.channel_samples[c % audio.channels][cut_end_frame + i];
                            spliced_channels[c][fade_start_idx + i] = old_sample * gain_out + new_sample * gain_in;
                        }
                    }
                    curr_frame = cut_end_frame + xfade_frames;
                } else {
                    curr_frame = cut_end_frame;
                }
            } else {
                // Find next cut boundary or end_frame
                let next_cut_start = cut_regions
                    .iter()
                    .filter(|r| r.start_seconds > curr_sec)
                    .map(|r| (r.start_seconds * sample_rate as f64) as usize)
                    .min()
                    .unwrap_or(end_frame)
                    .min(end_frame);

                if next_cut_start > curr_frame {
                    for c in 0..channels {
                        let in_c = c % audio.channels;
                        spliced_channels[c].extend_from_slice(&audio.channel_samples[in_c][curr_frame..next_cut_start]);
                    }
                    curr_frame = next_cut_start;
                } else {
                    break;
                }
            }
        }
    } else {
        for c in 0..channels {
            let in_c = c % audio.channels;
            spliced_channels[c].extend_from_slice(&audio.channel_samples[in_c][start_frame..end_frame]);
        }
    }

    let extracted_frames = spliced_channels[0].len();
    if extracted_frames == 0 {
        return Err("No audio frames remained after applying cut regions".to_string());
    }

    // 2. Pitch Shift & Speed Multiplier (Offline Signalsmith Stretch)
    let processed_channels: Vec<Vec<f32>> = if (config.bake_pitch && config.pitch_semitones.abs() > 0.001) || (config.bake_speed && (config.speed_multiplier - 1.0).abs() > 0.001) {
        let stretch = signalsmith_stretch_rs::SignalsmithStretch::new(channels, sample_rate as f32);
        if config.bake_pitch {
            stretch.set_transpose_semitones(config.pitch_semitones);
        }
        let speed = if config.bake_speed { config.speed_multiplier.clamp(0.25, 4.0) } else { 1.0 };
        
        let block_size = 4096;
        let mut out_channels = vec![Vec::new(); channels];

        let mut read_idx = 0;
        while read_idx < extracted_frames {
            let chunk_in_len = std::cmp::min(block_size, extracted_frames - read_idx);
            let chunk_out_len = ((chunk_in_len as f32) / speed).round() as usize;

            let mut in_scratch = vec![vec![0.0f32; chunk_in_len]; channels];
            for c in 0..channels {
                in_scratch[c].copy_from_slice(&spliced_channels[c][read_idx..read_idx + chunk_in_len]);
            }

            let mut out_scratch = vec![vec![0.0f32; chunk_out_len]; channels];

            let in_slices: Vec<&[f32]> = in_scratch.iter().map(|v| v.as_slice()).collect();
            let mut out_slices: Vec<&mut [f32]> = out_scratch.iter_mut().map(|v| v.as_mut_slice()).collect();

            stretch.process(&in_slices, &mut out_slices);

            for c in 0..channels {
                out_channels[c].extend_from_slice(&out_scratch[c]);
            }

            read_idx += chunk_in_len;
        }

        out_channels
    } else {
        spliced_channels
    };

    let final_frames = processed_channels[0].len();
    let mut final_data = processed_channels;

    // 3. High-Quality Biquad EQ Filtering
    if config.bake_eq && !config.eq_bands.is_empty() {
        let active_bands: Vec<&EqBand> = config.eq_bands.iter().filter(|b| b.enabled && (b.gain_db.abs() > 0.01 || matches!(b.filter_type, crate::dsp::FilterType::LowPass | crate::dsp::FilterType::HighPass | crate::dsp::FilterType::Notch))).collect();
        if !active_bands.is_empty() {
            let mut biquads: Vec<Biquad> = active_bands.iter().map(|b| {
                let mut bi = Biquad::new(channels);
                bi.set_params(b.filter_type, sample_rate as f64, b.freq, b.gain_db, b.q);
                bi
            }).collect();

            for f in 0..final_frames {
                for c in 0..channels {
                    let mut s = final_data[c][f];
                    for b in &mut biquads {
                        s = b.process_sample(c, s);
                    }
                    final_data[c][f] = s;
                }
            }
        }
    }

    // 4. Dual-Stage Dynamic Compressor
    if config.bake_compressor {
        let mut dual_compressor = DualCompressor::new(sample_rate as f64);
        dual_compressor.stage1.set_params(sample_rate as f64, config.comp_stage1);
        dual_compressor.stage2.set_params(sample_rate as f64, config.comp_stage2);
        dual_compressor.routing = config.comp_routing;
        dual_compressor.parallel_blend = config.comp_parallel_blend;

        if !dual_compressor.is_bypassed() {
            for f in 0..final_frames {
                let left_in = final_data[0][f];
                let right_in = if channels > 1 { final_data[1][f] } else { left_in };
                let (l_out, r_out) = dual_compressor.process_stereo_frame(left_in, right_in);
                final_data[0][f] = l_out;
                if channels > 1 {
                    final_data[1][f] = r_out;
                }
            }
        }
    }

    // 5. Volume Scaling
    let vol = config.volume_multiplier;
    if (vol - 1.0).abs() > 0.001 {
        for c in 0..channels {
            for s in &mut final_data[c] {
                *s *= vol;
            }
        }
    }

    // 6. Encode and Write to Disk (Hound WAV Encoder)
    let (bits, sample_format) = match config.bit_depth {
        ExportBitDepth::Int16 => (16, hound::SampleFormat::Int),
        ExportBitDepth::Int24 => (24, hound::SampleFormat::Int),
        ExportBitDepth::Float32 => (32, hound::SampleFormat::Float),
    };

    let spec = hound::WavSpec {
        channels: channels as u16,
        sample_rate: sample_rate as u32,
        bits_per_sample: bits,
        sample_format,
    };

    let mut writer = hound::WavWriter::create(Path::new(&config.output_path), spec)
        .map_err(|e| format!("Failed to create output WAV file: {}", e))?;

    match config.bit_depth {
        ExportBitDepth::Int16 => {
            for f in 0..final_frames {
                for c in 0..channels {
                    let s = final_data[c][f].clamp(-1.0, 1.0);
                    let sample_i16 = (s * 32767.0).round() as i16;
                    writer.write_sample(sample_i16).map_err(|e| e.to_string())?;
                }
            }
        }
        ExportBitDepth::Int24 => {
            for f in 0..final_frames {
                for c in 0..channels {
                    let s = final_data[c][f].clamp(-1.0, 1.0);
                    // 24-bit PCM scaled into signed 32-bit int
                    let sample_i24 = (s * 8388607.0).round() as i32;
                    writer.write_sample(sample_i24).map_err(|e| e.to_string())?;
                }
            }
        }
        ExportBitDepth::Float32 => {
            for f in 0..final_frames {
                for c in 0..channels {
                    let s = final_data[c][f];
                    writer.write_sample(s).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    writer.finalize().map_err(|e| format!("Failed to finalize WAV file: {}", e))?;

    Ok(final_frames)
}
