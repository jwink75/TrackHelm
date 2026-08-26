pub mod websocket;
pub mod osc;
pub mod midi;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "data")]
pub enum RemoteCommand {
    #[serde(rename = "play")]
    Play,
    #[serde(rename = "pause")]
    Pause,
    #[serde(rename = "play_pause")]
    PlayPause,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "rewind")]
    Rewind,
    #[serde(rename = "next_track")]
    NextTrack,
    #[serde(rename = "prev_track")]
    PrevTrack,
    #[serde(rename = "select_track")]
    SelectTrack { index: usize },
    #[serde(rename = "next_marker")]
    NextMarker,
    #[serde(rename = "prev_marker")]
    PrevMarker,
    #[serde(rename = "add_marker")]
    AddMarker,
    #[serde(rename = "pitch_up")]
    PitchUp,
    #[serde(rename = "pitch_down")]
    PitchDown,
    #[serde(rename = "adjust_pitch")]
    AdjustPitch { delta: f32 },
    #[serde(rename = "set_pitch")]
    SetPitch { semitones: f32 },
    #[serde(rename = "volume_up")]
    VolumeUp,
    #[serde(rename = "volume_down")]
    VolumeDown,
    #[serde(rename = "adjust_volume")]
    AdjustVolume { delta: f32 },
    #[serde(rename = "set_volume")]
    SetVolume { db: f32 },
    #[serde(rename = "speed_up")]
    SpeedUp,
    #[serde(rename = "speed_down")]
    SpeedDown,
    #[serde(rename = "adjust_speed")]
    AdjustSpeed { delta: f32 },
    #[serde(rename = "set_speed")]
    SetSpeed { speed: f32 },
    #[serde(rename = "toggle_loop")]
    ToggleLoop,
    #[serde(rename = "toggle_cut")]
    ToggleCut,
    #[serde(rename = "get_status")]
    GetStatus,
}
