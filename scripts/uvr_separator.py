#!/usr/bin/env python3
"""
TrackHelm UVR Stem Separation Worker
------------------------------------
Executes Ultimate Vocal Remover models via audio-separator with Apple Silicon
acceleration (MPS / CoreML), native macOS AAC encoding (afconvert), and real-time
progress streaming.

Supported Modes:
  1. ensemble_vocals: Runs 4-model ensemble (Kim Vocal 2, MDX23C-InstVoc HQ,
     UVR-MDX-NET-Voc_FT, htdemucs_ft) with Max-Spec blending, producing:
     "<Track> (Vocals Ensemble).m4a" in a "Vocals Only" folder.
  2. lead_backups: Runs VR Arch 5_HP-Karaoke-UVR on a vocal track, producing:
     "<Track> (Lead Vocals).m4a" and "<Track> (Backing Vocals).m4a".
"""

import sys
import os
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

def emit_event(event_type, **kwargs):
    payload = {"type": event_type, **kwargs}
    print(json.dumps(payload), flush=True)

def ensure_model_symlinks(cache_dir: Path) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    uvr_root = Path("/Applications/Ultimate Vocal Remover.app/Contents/Resources/models")
    if not uvr_root.exists():
        emit_event("log", message=f"Warning: UVR application models not found at {uvr_root}")
        return cache_dir

    # Symlink files recursively so audio-separator can resolve any model without network download
    for root, dirs, files in os.walk(uvr_root):
        for f in files:
            src = Path(root) / f
            dest = cache_dir / f
            if not dest.exists():
                try:
                    dest.symlink_to(src)
                except Exception:
                    pass
    return cache_dir

def convert_wav_to_aac(wav_path: str, m4a_path: str, bitrate: int = 256000) -> bool:
    """Uses native macOS afconvert to encode AAC .m4a."""
    cmd = [
        "/usr/bin/afconvert",
        "-f", "m4af",
        "-d", "aac",
        "-b", str(bitrate),
        wav_path,
        m4a_path
    ]
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        emit_event("log", message=f"afconvert error: {res.stderr}")
        return False
    return os.path.exists(m4a_path)

def clean_base_name(filename: str) -> str:
    base = Path(filename).stem
    # Strip any trailing (Vocals Ensemble) or (Vocals) if chaining lead/backups
    for suffix in [" (Vocals Ensemble)", " (Vocals)", " (Instrumental)", "_Vocals", "_Instrumental"]:
        if base.endswith(suffix):
            base = base[:-len(suffix)]
    return base

def run_ensemble_vocals(input_path: Path, output_dir: Path, cache_dir: Path):
    import logging
    from audio_separator.separator import Separator

    emit_event("progress", percent=5, stage="Initializing 4-Model Ensemble Engine...")

    models = [
        "Kim_Vocal_2.onnx",
        "MDX23C-8KFFT-InstVoc_HQ.ckpt",
        "UVR-MDX-NET-Voc_FT.onnx",
        "htdemucs_ft.yaml"
    ]

    class UvrLogHandler(logging.Handler):
        def emit(self, record):
            try:
                msg = record.getMessage()
                if "Processing with model: Kim_Vocal_2" in msg:
                    emit_event("progress", percent=20, stage="Model 1/4: Kim Vocal 2 (MDX-Net)...")
                elif "Processing with model: MDX23C" in msg:
                    emit_event("progress", percent=40, stage="Model 2/4: MDX23C-InstVoc HQ (MDXC)...")
                elif "Processing with model: UVR-MDX-NET-Voc_FT" in msg:
                    emit_event("progress", percent=60, stage="Model 3/4: UVR-MDX-NET-Voc_FT (MDX-Net)...")
                elif "Processing with model: htdemucs_ft" in msg:
                    emit_event("progress", percent=80, stage="Model 4/4: Demucs v4 (htdemucs_ft)...")
                elif "Ensembling" in msg:
                    emit_event("progress", percent=90, stage="Blending Vocals with Max-Spec Spectrogram...")
            except Exception:
                pass

    # Create temporary directory for intermediate stems
    with tempfile.TemporaryDirectory(prefix="trackhelm_uvr_") as temp_dir:
        temp_dir_path = Path(temp_dir)
        
        separator = Separator(
            model_file_dir=str(cache_dir),
            output_dir=str(temp_dir_path),
            output_format="WAV",
            output_single_stem="Vocals",
            ensemble_algorithm="uvr_max_spec",
            log_level=20 # INFO
        )
        handler = UvrLogHandler()
        separator.logger.addHandler(handler)
        logging.getLogger("audio_separator").addHandler(handler)

        emit_event("progress", percent=10, stage="Configuring Models & GPU Acceleration...")
        separator.load_model(models)

        emit_event("progress", percent=15, stage="Model 1/4: Kim Vocal 2 (MDX-Net)...")
        
        # Run separation
        output_stems = separator.separate(str(input_path))
        
        emit_event("progress", percent=95, stage="Ensemble Complete. Encoding pristine AAC...")

        if not output_stems:
            raise RuntimeError("Separation did not produce any output stems")

        # The master ensemble output wav
        ensemble_wav = output_stems[0]
        if not os.path.isabs(ensemble_wav):
            ensemble_wav = str(temp_dir_path / ensemble_wav)

        # Output target setup
        output_dir.mkdir(parents=True, exist_ok=True)
        base_title = clean_base_name(input_path.name)
        out_m4a_name = f"{base_title} (Vocals Ensemble).m4a"
        out_m4a_path = output_dir / out_m4a_name

        success = convert_wav_to_aac(ensemble_wav, str(out_m4a_path))
        if not success:
            raise RuntimeError("Failed to encode output stem to AAC using afconvert")

        emit_event("progress", percent=100, stage="Done!")
        emit_event("complete", success=True, files=[{
            "name": out_m4a_name,
            "path": str(out_m4a_path),
            "role": "vocals",
            "fileType": "audio"
        }])

def run_lead_backups(input_path: Path, output_dir: Path, cache_dir: Path):
    import logging
    from audio_separator.separator import Separator

    emit_event("progress", percent=10, stage="Initializing 5HP Karaoke Model (VR Arch)...")

    class KaraokeLogHandler(logging.Handler):
        def emit(self, record):
            try:
                msg = record.getMessage()
                if "Starting separation process" in msg:
                    emit_event("progress", percent=35, stage="Processing Vocals with 5HP Karaoke...")
                elif "Saving Instrumental stem" in msg or "Saving Vocals stem" in msg:
                    emit_event("progress", percent=75, stage="Extracting Lead and Backing Vocals...")
            except Exception:
                pass

    with tempfile.TemporaryDirectory(prefix="trackhelm_5hp_") as temp_dir:
        temp_dir_path = Path(temp_dir)

        separator = Separator(
            model_file_dir=str(cache_dir),
            output_dir=str(temp_dir_path),
            output_format="WAV",
            log_level=20
        )
        handler = KaraokeLogHandler()
        separator.logger.addHandler(handler)
        logging.getLogger("audio_separator").addHandler(handler)

        separator.load_model("5_HP-Karaoke-UVR.pth")
        emit_event("progress", percent=30, stage="Separating Lead and Backing Vocals...")


        output_stems = separator.separate(str(input_path))
        emit_event("progress", percent=80, stage="Converting Stems to AAC...")

        lead_wav = None
        backings_wav = None

        for stem in output_stems:
            stem_full = str(temp_dir_path / stem) if not os.path.isabs(stem) else stem
            if "(Vocals)" in stem:
                lead_wav = stem_full
            elif "(Instrumental)" in stem:
                backings_wav = stem_full

        output_dir.mkdir(parents=True, exist_ok=True)
        base_title = clean_base_name(input_path.name)

        results = []

        if lead_wav and os.path.exists(lead_wav):
            lead_name = f"{base_title} (Lead Vocals).m4a"
            lead_path = output_dir / lead_name
            if convert_wav_to_aac(lead_wav, str(lead_path)):
                results.append({
                    "name": lead_name,
                    "path": str(lead_path),
                    "role": "lead",
                    "fileType": "audio"
                })

        if backings_wav and os.path.exists(backings_wav):
            backings_name = f"{base_title} (Backing Vocals).m4a"
            backings_path = output_dir / backings_name
            if convert_wav_to_aac(backings_wav, str(backings_path)):
                results.append({
                    "name": backings_name,
                    "path": str(backings_path),
                    "role": "backings",
                    "fileType": "audio"
                })

        if not results:
            raise RuntimeError("Failed to generate lead/backing stems from 5HP Karaoke model")

        emit_event("progress", percent=100, stage="Done!")
        emit_event("complete", success=True, files=results)

def main():
    parser = argparse.ArgumentParser(description="TrackHelm UVR Stem Separator")
    parser.add_argument("--input", required=True, help="Input audio file path")
    parser.add_argument("--mode", required=True, choices=["ensemble_vocals", "lead_backups"], help="Separation mode")
    parser.add_argument("--output-dir", help="Output directory path (defaults to [InputDir]/Vocals Only)")
    args = parser.parse_args()

    input_path = Path(args.input).resolve()
    if not input_path.exists():
        emit_event("error", message=f"Input file does not exist: {args.input}")
        sys.exit(1)

    if args.output_dir:
        output_dir = Path(args.output_dir).resolve()
    else:
        # If input is already in a "Vocals Only" folder, keep output in that same folder
        if input_path.parent.name.lower() == "vocals only":
            output_dir = input_path.parent
        else:
            output_dir = input_path.parent / "Vocals Only"

    project_root = Path(__file__).resolve().parent.parent
    cache_dir = project_root / ".models_cache"
    ensure_model_symlinks(cache_dir)

    try:
        if args.mode == "ensemble_vocals":
            run_ensemble_vocals(input_path, output_dir, cache_dir)
        elif args.mode == "lead_backups":
            run_lead_backups(input_path, output_dir, cache_dir)
    except Exception as e:
        emit_event("error", message=str(e))
        sys.exit(1)

if __name__ == "__main__":
    main()
