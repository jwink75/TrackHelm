use tauri::{AppHandle, Emitter, Runtime};
use tokio::net::UdpSocket;
use rosc::{OscPacket, OscType};

pub fn start_osc_server<R: Runtime>(app: AppHandle<R>, port: u16) {
    tauri::async_runtime::spawn(async move {
        let addr = format!("0.0.0.0:{}", port);
        let socket = match UdpSocket::bind(&addr).await {
            Ok(s) => {
                println!("[OSC Server] Listening on UDP {}", addr);
                s
            }
            Err(e) => {
                eprintln!("[OSC Server] Failed to bind UDP {}: {}", addr, e);
                return;
            }
        };

        let mut buf = [0u8; rosc::decoder::MTU];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((size, _peer)) => {
                    if let Ok((_remaining, packet)) = rosc::decoder::decode_udp(&buf[..size]) {
                        handle_osc_packet(&app, packet);
                    }
                }
                Err(e) => {
                    eprintln!("[OSC Server] Error receiving UDP packet: {}", e);
                }
            }
        }
    });
}

fn handle_osc_packet<R: Runtime>(app: &AppHandle<R>, packet: OscPacket) {
    match packet {
        OscPacket::Message(msg) => {
            let addr = msg.addr.as_str();
            let payload = match addr {
                "/trackhelm/playpause" | "/trackhelm/play_pause" => Some(r#"{"action":"play_pause"}"#.to_string()),
                "/trackhelm/play" => Some(r#"{"action":"play"}"#.to_string()),
                "/trackhelm/pause" => Some(r#"{"action":"pause"}"#.to_string()),
                "/trackhelm/stop" => Some(r#"{"action":"stop"}"#.to_string()),
                "/trackhelm/rewind" => Some(r#"{"action":"rewind"}"#.to_string()),
                "/trackhelm/track/next" | "/trackhelm/next_track" => Some(r#"{"action":"next_track"}"#.to_string()),
                "/trackhelm/track/prev" | "/trackhelm/prev_track" => Some(r#"{"action":"prev_track"}"#.to_string()),
                "/trackhelm/marker/next" | "/trackhelm/next_marker" => Some(r#"{"action":"next_marker"}"#.to_string()),
                "/trackhelm/marker/prev" | "/trackhelm/prev_marker" => Some(r#"{"action":"prev_marker"}"#.to_string()),
                "/trackhelm/marker/add" | "/trackhelm/add_marker" => Some(r#"{"action":"add_marker"}"#.to_string()),
                "/trackhelm/pitch/inc" | "/trackhelm/pitch/up" => Some(r#"{"action":"pitch_up"}"#.to_string()),
                "/trackhelm/pitch/dec" | "/trackhelm/pitch/down" => Some(r#"{"action":"pitch_down"}"#.to_string()),
                "/trackhelm/pitch" => {
                    if let Some(OscType::Float(v)) = msg.args.first() {
                        Some(format!(r#"{{"action":"set_pitch","data":{{"semitones":{}}}}}"#, v))
                    } else if let Some(OscType::Int(v)) = msg.args.first() {
                        Some(format!(r#"{{"action":"set_pitch","data":{{"semitones":{}}}}}"#, *v as f32))
                    } else {
                        None
                    }
                }
                "/trackhelm/volume/inc" | "/trackhelm/volume/up" => Some(r#"{"action":"volume_up"}"#.to_string()),
                "/trackhelm/volume/dec" | "/trackhelm/volume/down" => Some(r#"{"action":"volume_down"}"#.to_string()),
                "/trackhelm/volume" => {
                    if let Some(OscType::Float(v)) = msg.args.first() {
                        Some(format!(r#"{{"action":"set_volume","data":{{"db":{}}}}}"#, v))
                    } else if let Some(OscType::Int(v)) = msg.args.first() {
                        Some(format!(r#"{{"action":"set_volume","data":{{"db":{}}}}}"#, *v as f32))
                    } else {
                        None
                    }
                }
                "/trackhelm/speed/inc" | "/trackhelm/speed/up" => Some(r#"{"action":"speed_up"}"#.to_string()),
                "/trackhelm/speed/dec" | "/trackhelm/speed/down" => Some(r#"{"action":"speed_down"}"#.to_string()),
                "/trackhelm/speed" => {
                    if let Some(OscType::Float(v)) = msg.args.first() {
                        Some(format!(r#"{{"action":"set_speed","data":{{"speed":{}}}}}"#, v))
                    } else {
                        None
                    }
                }
                "/trackhelm/loop" | "/trackhelm/toggle_loop" => Some(r#"{"action":"toggle_loop"}"#.to_string()),
                "/trackhelm/cut" | "/trackhelm/toggle_cut" => Some(r#"{"action":"toggle_cut"}"#.to_string()),
                _ => None,
            };

            if let Some(p) = payload {
                let _ = app.emit("remote-control-action", p);
            }
        }
        OscPacket::Bundle(bundle) => {
            for packet in bundle.content {
                handle_osc_packet(app, packet);
            }
        }
    }
}
