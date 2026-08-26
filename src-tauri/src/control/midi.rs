use midir::{MidiInput, MidiInputConnection};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

pub struct MidiManager {
    connection: Mutex<Option<MidiInputConnection<()>>>,
}

impl MidiManager {
    pub fn new() -> Self {
        Self {
            connection: Mutex::new(None),
        }
    }

    pub fn list_ports(&self) -> Vec<String> {
        let midi_in = match MidiInput::new("TrackHelm Port Scanner") {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };

        let mut ports = Vec::new();
        for p in &midi_in.ports() {
            if let Ok(name) = midi_in.port_name(p) {
                ports.push(name);
            }
        }
        ports
    }

    pub fn connect_port<R: Runtime>(&self, app: AppHandle<R>, port_name_or_index: &str) -> Result<String, String> {
        let mut midi_in = MidiInput::new("TrackHelm MIDI Listener")
            .map_err(|e| format!("Failed to create MIDI input: {}", e))?;
        midi_in.ignore(midir::Ignore::None);

        let ports = midi_in.ports();
        let target_port = if let Ok(idx) = port_name_or_index.parse::<usize>() {
            ports.get(idx).cloned()
        } else {
            ports.into_iter().find(|p| {
                midi_in.port_name(p).map(|name| name == port_name_or_index).unwrap_or(false)
            })
        };

        let port = match target_port {
            Some(p) => p,
            None => return Err("MIDI port not found".to_string()),
        };

        let connected_name = midi_in.port_name(&port).unwrap_or_else(|_| "Unknown Port".to_string());

        let conn = midi_in
            .connect(
                &port,
                "trackhelm-in",
                move |_stamp, message, _| {
                    handle_midi_bytes(&app, message);
                },
                (),
            )
            .map_err(|e| format!("Failed to connect to MIDI port: {}", e))?;

        let mut guard = self.connection.lock().unwrap();
        *guard = Some(conn);

        Ok(connected_name)
    }

    pub fn disconnect(&self) {
        let mut guard = self.connection.lock().unwrap();
        *guard = None;
    }
}

fn handle_midi_bytes<R: Runtime>(app: &AppHandle<R>, msg: &[u8]) {
    if msg.is_empty() {
        return;
    }

    let status = msg[0] & 0xF0;
    let _channel = msg[0] & 0x0F;

    match status {
        // Note On
        0x90 => {
            if msg.len() >= 3 {
                let note = msg[1];
                let velocity = msg[2];
                if velocity > 0 {
                    let payload = format!(r#"{{"type":"note_on","note":{},"velocity":{}}}"#, note, velocity);
                    let _ = app.emit("midi-event", payload);
                }
            }
        }
        // Control Change (CC)
        0xB0 => {
            if msg.len() >= 3 {
                let cc = msg[1];
                let val = msg[2];
                let payload = format!(r#"{{"type":"cc","cc":{},"value":{}}}"#, cc, val);
                let _ = app.emit("midi-event", payload);
            }
        }
        // Program Change
        0xC0 => {
            if msg.len() >= 2 {
                let program = msg[1];
                let payload = format!(r#"{{"type":"program_change","program":{}}}"#, program);
                let _ = app.emit("midi-event", payload);
            }
        }
        _ => {}
    }
}
