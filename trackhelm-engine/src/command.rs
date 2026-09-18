use std::sync::Arc;
use std::time::Duration;
use crate::decoder::DecodedAudio;

pub const MAX_EQ_BANDS: usize = 16;
pub const MAX_ENGINE_REGIONS: usize = 32;
pub const MAX_ENVELOPE_NODES: usize = 64;

#[derive(Copy, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeCurve {
    Linear,
    CurveUp,
    CurveDown,
    Sine,
}

impl Default for EnvelopeCurve {
    fn default() -> Self {
        EnvelopeCurve::Linear
    }
}

#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeNode {
    #[serde(alias = "time_seconds")]
    pub time_seconds: f64,
    #[serde(alias = "gain_db")]
    pub gain_db: f64,
    #[serde(default)]
    pub curve: EnvelopeCurve,
}

impl Default for EnvelopeNode {
    fn default() -> Self {
        Self {
            time_seconds: 0.0,
            gain_db: 0.0,
            curve: EnvelopeCurve::Linear,
        }
    }
}

pub fn interpolate_envelope(nodes: &[EnvelopeNode], time_seconds: f64) -> f32 {
    if nodes.is_empty() {
        return 1.0;
    }

    if time_seconds <= nodes[0].time_seconds {
        let db = nodes[0].gain_db;
        return if db <= -59.5 { 0.0 } else { 10.0f64.powf(db / 20.0) as f32 };
    }

    let last_idx = nodes.len() - 1;
    if time_seconds >= nodes[last_idx].time_seconds {
        let db = nodes[last_idx].gain_db;
        return if db <= -59.5 { 0.0 } else { 10.0f64.powf(db / 20.0) as f32 };
    }

    for i in 0..last_idx {
        let a = &nodes[i];
        let b = &nodes[i + 1];
        if time_seconds >= a.time_seconds && time_seconds <= b.time_seconds {
            let seg_len = (b.time_seconds - a.time_seconds).max(1e-6);
            let t = ((time_seconds - a.time_seconds) / seg_len).clamp(0.0, 1.0);

            let shaped_t = match a.curve {
                EnvelopeCurve::Linear => t,
                EnvelopeCurve::CurveUp => t * t,
                EnvelopeCurve::CurveDown => 1.0 - (1.0 - t) * (1.0 - t),
                EnvelopeCurve::Sine => 0.5 * (1.0 - (std::f64::consts::PI * t).cos()),
            };

            let db = a.gain_db + (b.gain_db - a.gain_db) * shaped_t;
            return if db <= -59.5 { 0.0 } else { 10.0f64.powf(db / 20.0) as f32 };
        }
    }

    1.0
}

#[derive(Copy, Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRegion {
    #[serde(alias = "start_seconds")]
    pub start_seconds: f64,
    #[serde(alias = "end_seconds")]
    pub end_seconds: f64,
    #[serde(alias = "is_loop")]
    pub is_loop: bool,
    #[serde(alias = "is_cut")]
    pub is_cut: bool,
    #[serde(default = "default_crossfade_ms", alias = "crossfade_ms")]
    pub crossfade_ms: f64,
}

fn default_crossfade_ms() -> f64 {
    5.0
}

#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    #[serde(alias = "filter_type")]
    pub filter_type: crate::dsp::FilterType,
    pub freq: f64,
    #[serde(alias = "gain_db")]
    pub gain_db: f64,
    pub q: f64,
    pub enabled: bool,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            filter_type: crate::dsp::FilterType::Peaking,
            freq: 1000.0,
            gain_db: 0.0,
            q: 0.707,
            enabled: false,
        }
    }
}

pub enum Command {
    Play,
    Pause,
    Stop,
    StopBackgroundTracks,
    Seek(Duration),
    SetPitch(f32), // In semitones
    SetTempo(f32), // Playback speed multiplier
    SetVolume(f32), // 0.0 to 1.0+
    LoadAudio(Arc<DecodedAudio>),
    SetEq { bass_db: f32, mid_db: f32, treble_db: f32 },
    SetEqBands([EqBand; MAX_EQ_BANDS], usize),
    SetCompressor { threshold_db: f32, ratio: f32, makeup_db: f32, attack_ms: f32, release_ms: f32 },
    SetDualCompressor {
        stage1: crate::dsp::CompStageParams,
        stage2: crate::dsp::CompStageParams,
        routing: crate::dsp::CompRouting,
        parallel_blend: f32,
    },
    SetRegions([EngineRegion; MAX_ENGINE_REGIONS], usize),
    SetVolumeEnvelope([EnvelopeNode; MAX_ENVELOPE_NODES], usize),
}

pub struct CommandBus {
    sender: crossbeam_channel::Sender<Command>,
}

impl CommandBus {
    pub fn new(sender: crossbeam_channel::Sender<Command>) -> Self {
        Self { sender }
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.sender
            .send(command)
            .map_err(|e| format!("Failed to send command: {:?}", e))
    }
}
