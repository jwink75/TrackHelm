fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(
                tauri_build::AppManifest::new()
                    .commands(&[
                        "load_track",
                        "preload_track",
                        "play",
                        "pause",
                        "stop",
                        "seek",
                        "set_volume",
                        "set_speed",
                        "set_pitch",
                        "get_playback_status",
                        "read_dir",
                        "get_waveform_slice",
                        "get_raw_samples",
                        "get_cloud_folders"
                    ])
            )
    ).expect("failed to run tauri-build");
}
