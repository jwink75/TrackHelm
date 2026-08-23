use crossbeam_channel::{unbounded, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Command {
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetPitch(f32), // In semitones or multiplier
    SetTempo(f32), // Playback rate multiplier
    SetVolume(f32), // 0.0 to 1.0+
    LoadFile(String), // Path to target audio file
}

pub struct CommandBus {
    sender: Sender<Command>,
}

impl CommandBus {
    pub fn new() -> (Self, Receiver<Command>) {
        let (sender, receiver) = unbounded();
        (Self { sender }, receiver)
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.sender
            .send(command)
            .map_err(|e| format!("Failed to send command: {:?}", e))
    }
}
