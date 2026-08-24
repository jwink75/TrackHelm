use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::FromSample;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

pub struct DecodedAudio {
    pub channels: usize,
    pub sample_rate: u32,
    pub duration_seconds: f64,
    pub channel_samples: Vec<Vec<f32>>,
}

pub fn decode_file<P: AsRef<Path>>(path: P) -> Result<DecodedAudio, String> {
    let path_ref = path.as_ref();
    let file = File::open(path_ref).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss_opts = MediaSourceStreamOptions {
        buffer_len: 128 * 1024,
    };
    let mss = MediaSourceStream::new(Box::new(file), mss_opts);

    let mut hint = Hint::new();
    if let Some(ext) = path_ref.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed = get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("Unsupported file format: {}", e))?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_ok_or_else(|| "No audio track found in file".to_string())?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let mut channel_samples: Vec<Vec<f32>> = Vec::new();
    let mut sample_rate = 0;
    let mut channels = 0;

    // Decode loop
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(_)) => break, // End of file
            Err(e) => return Err(format!("Error reading packet: {}", e)),
        };

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::DecodeError(e)) => {
                log::warn!("Decode packet error: {}, skipping packet", e);
                continue;
            }
            Err(e) => return Err(format!("Failed to decode packet: {}", e)),
        };

        // Initialize parameters from the first decoded buffer spec if not already set
        if channels == 0 {
            let spec = decoded.spec();
            channels = spec.channels.count();
            sample_rate = spec.rate;
            channel_samples = vec![Vec::with_capacity(500_000); channels];
        }

        copy_samples_to_f32(&decoded, &mut channel_samples);
    }

    if channels == 0 || channel_samples.is_empty() || channel_samples[0].is_empty() {
        return Err("No audio samples decoded".to_string());
    }

    let total_samples = channel_samples[0].len();
    let duration_seconds = total_samples as f64 / sample_rate as f64;

    Ok(DecodedAudio {
        channels,
        sample_rate,
        duration_seconds,
        channel_samples,
    })
}

// Convert all symphonia buffer types to f32 samples using FromSample trait
fn copy_samples_to_f32(buf: &AudioBufferRef, dest: &mut Vec<Vec<f32>>) {
    match buf {
        AudioBufferRef::U8(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::U16(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::S8(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::S16(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::S24(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::S32(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::U24(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::U32(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
        AudioBufferRef::F32(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                dest[c].extend_from_slice(chan);
            }
        }
        AudioBufferRef::F64(buf) => {
            let channels = buf.spec().channels.count();
            for c in 0..channels {
                let chan = buf.chan(c);
                for &sample in chan {
                    dest[c].push(f32::from_sample(sample));
                }
            }
        }
    }
}

trait OptionExt<T> {
    fn ok_ok_or_else<F: FnOnce() -> String>(self, err: F) -> Result<T, String>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_ok_or_else<F: FnOnce() -> String>(self, err: F) -> Result<T, String> {
        self.ok_or_else(err)
    }
}
