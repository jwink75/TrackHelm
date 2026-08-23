use crate::command::Command;
use crossbeam_channel::Receiver;

pub struct AudioEngine {
    _command_receiver: Receiver<Command>,
    is_playing: bool,
}

impl AudioEngine {
    pub fn new(command_receiver: Receiver<Command>) -> Self {
        Self {
            _command_receiver: command_receiver,
            is_playing: false,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        // CPAL output stream initialization placeholder
        log::info!("Audio engine standby.");
        Ok(())
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
}
