fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(
                tauri_build::AppManifest::new()
                    .commands(&[
                        "load_track",
                        "play",
                        "pause",
                        "stop",
                        "seek",
                        "set_volume",
                        "get_playback_status",
                        "read_dir",
                        "get_waveform_slice"
                    ])
            )
    ).expect("failed to run tauri-build");
}
