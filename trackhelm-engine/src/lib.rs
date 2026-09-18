pub mod command;
pub mod engine;
pub mod graph;
pub mod db;
pub mod decoder;
pub mod dsp;
pub mod export;
pub mod resampler;

// Re-export key components
pub use command::{Command, CommandBus, EngineRegion, EnvelopeNode, EnvelopeCurve, MAX_ENVELOPE_NODES, interpolate_envelope};
pub use engine::{AudioEngine, SharedEngineState, AudioDeviceItem};
pub use graph::AudioGraph;
pub use db::Database;
pub use decoder::{decode_file, DecodedAudio};
pub use export::{render_audio_export, ExportAudioConfig, ExportBitDepth};
pub use resampler::resample_audio_channels;
