use rubato::{Resampler, FftFixedInOut};

pub fn resample_audio_channels(
    channels: &[Vec<f32>],
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<Vec<f32>>, String> {
    if channels.is_empty() || channels[0].is_empty() {
        return Ok(channels.to_vec());
    }

    if from_rate == to_rate {
        return Ok(channels.to_vec());
    }

    let num_channels = channels.len();
    let in_len = channels[0].len();

    let chunk_size = 1024;
    let mut resampler = FftFixedInOut::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        num_channels,
    ).map_err(|e| format!("Failed to create resampler: {}", e))?;

    let in_frames_needed = resampler.input_frames_next();
    let mut output_channels: Vec<Vec<f32>> = vec![Vec::new(); num_channels];

    let mut pos = 0;
    let mut scratch_in: Vec<Vec<f32>> = vec![vec![0.0f32; in_frames_needed]; num_channels];

    while pos < in_len {
        let frames_to_copy = (in_len - pos).min(in_frames_needed);
        for ch in 0..num_channels {
            scratch_in[ch][..frames_to_copy].copy_from_slice(&channels[ch][pos..pos + frames_to_copy]);
            if frames_to_copy < in_frames_needed {
                scratch_in[ch][frames_to_copy..].fill(0.0);
            }
        }

        let out = resampler.process(&scratch_in, None)
            .map_err(|e| format!("Resampling process error: {}", e))?;

        for ch in 0..num_channels {
            output_channels[ch].extend_from_slice(&out[ch]);
        }

        pos += frames_to_copy;
        if frames_to_copy < in_frames_needed {
            break;
        }
    }

    let delay = resampler.output_delay();
    // Flush extra zero chunks if needed to push all input through the filter
    let extra_chunks_needed = (delay + resampler.output_frames_next() - 1) / resampler.output_frames_next() + 1;
    scratch_in.iter_mut().for_each(|ch| ch.fill(0.0));
    for _ in 0..extra_chunks_needed {
        let out = resampler.process(&scratch_in, None)
            .map_err(|e| format!("Resampling flush error: {}", e))?;
        for ch in 0..num_channels {
            output_channels[ch].extend_from_slice(&out[ch]);
        }
    }

    // Trim output: skip initial filter delay and take exactly expected_total_frames
    let expected_total_frames = ((in_len as f64 * to_rate as f64) / from_rate as f64).round() as usize;
    let mut final_channels = vec![Vec::with_capacity(expected_total_frames); num_channels];
    for ch in 0..num_channels {
        if output_channels[ch].len() > delay {
            let available = output_channels[ch].len() - delay;
            let take = available.min(expected_total_frames);
            final_channels[ch].extend_from_slice(&output_channels[ch][delay..delay + take]);
        }
        // If slightly short due to rounding, pad with 0
        if final_channels[ch].len() < expected_total_frames {
            final_channels[ch].resize(expected_total_frames, 0.0);
        }
    }

    Ok(final_channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_44100_to_48000_sine() {
        let from_rate = 44100;
        let to_rate = 48000;
        let freq = 440.0;
        let duration_sec = 1.0;
        let in_samples = (duration_sec * from_rate as f64) as usize;

        // Generate 440 Hz sine wave at 44.1 kHz
        let mut mono = Vec::with_capacity(in_samples);
        for i in 0..in_samples {
            let t = i as f64 / from_rate as f64;
            mono.push((2.0 * std::f64::consts::PI * freq * t).sin() as f32);
        }

        let input = vec![mono];
        let output = resample_audio_channels(&input, from_rate, to_rate).expect("Resampling failed");

        assert_eq!(output.len(), 1);
        let expected_out_samples = (duration_sec * to_rate as f64).round() as usize;
        assert_eq!(output[0].len(), expected_out_samples);

        // Check phase and value at a middle sample (after any filter startup transients)
        let test_idx = to_rate as usize / 2; // at 0.5s
        let t_out = test_idx as f64 / to_rate as f64;
        let expected_val = (2.0 * std::f64::consts::PI * freq * t_out).sin() as f32;
        let actual_val = output[0][test_idx];
        assert!((actual_val - expected_val).abs() < 0.05, "Value at 0.5s mismatch: actual {}, expected {}", actual_val, expected_val);
    }

    #[test]
    fn test_resample_48000_to_44100_sine() {
        let from_rate = 48000;
        let to_rate = 44100;
        let freq = 1000.0;
        let duration_sec = 0.5;
        let in_samples = (duration_sec * from_rate as f64) as usize;

        let mut mono = Vec::with_capacity(in_samples);
        for i in 0..in_samples {
            let t = i as f64 / from_rate as f64;
            mono.push((2.0 * std::f64::consts::PI * freq * t).sin() as f32);
        }

        let input = vec![mono];
        let output = resample_audio_channels(&input, from_rate, to_rate).expect("Downsampling failed");

        let expected_out_samples = (duration_sec * to_rate as f64).round() as usize;
        assert_eq!(output[0].len(), expected_out_samples);

        let test_idx = to_rate as usize / 4; // at 0.25s
        let t_out = test_idx as f64 / to_rate as f64;
        let expected_val = (2.0 * std::f64::consts::PI * freq * t_out).sin() as f32;
        let actual_val = output[0][test_idx];
        assert!((actual_val - expected_val).abs() < 0.1, "Value at 0.25s mismatch: actual {}, expected {}", actual_val, expected_val);
    }

    #[test]
    fn test_identity_resample() {
        let data = vec![vec![0.1f32, 0.2, 0.3, 0.4]];
        let out = resample_audio_channels(&data, 44100, 44100).unwrap();
        assert_eq!(data, out);
    }
}
