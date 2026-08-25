use std::sync::Arc;
use std::time::Duration;
use crate::decoder::DecodedAudio;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRegion {
    pub id: String,
    pub name: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub is_loop: bool,
    pub is_cut: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    pub filter_type: crate::dsp::FilterType,
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
    pub enabled: bool,
}

pub enum Command {
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetPitch(f32), // In semitones
    SetTempo(f32), // Playback speed multiplier
    SetVolume(f32), // 0.0 to 1.0+
    LoadAudio(Arc<DecodedAudio>),
    SetEq { bass_db: f32, mid_db: f32, treble_db: f32 },
    SetEqBands(Vec<EqBand>),
    SetCompressor { threshold_db: f32, ratio: f32, makeup_db: f32, attack_ms: f32, release_ms: f32 },
    SetDualCompressor {
        stage1: crate::dsp::CompStageParams,
        stage2: crate::dsp::CompStageParams,
        routing: crate::dsp::CompRouting,
        parallel_blend: f32,
    },
    SetRegions(Vec<EngineRegion>),
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
