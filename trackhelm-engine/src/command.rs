use std::sync::Arc;
use std::time::Duration;
use crate::decoder::DecodedAudio;

pub enum Command {
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetPitch(f32), // In semitones
    SetTempo(f32), // Playback speed multiplier
    SetVolume(f32), // 0.0 to 1.0+
    LoadAudio(Arc<DecodedAudio>),
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
