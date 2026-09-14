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

# On Windows, explicitly register PyTorch and CUDA runtime DLL directories
if sys.platform == "win32":
    try:
        import torch
        torch_lib = os.path.join(os.path.dirname(torch.__file__), "lib")
        if os.path.isdir(torch_lib):
            os.add_dll_directory(torch_lib)
            os.environ["PATH"] = torch_lib + os.pathsep + os.environ.get("PATH", "")
    except Exception:
        pass
    
    cuda_path = os.environ.get("CUDA_PATH")
    if cuda_path:
        cuda_bin = os.path.join(cuda_path, "bin")
        if os.path.isdir(cuda_bin):
            try:
                os.add_dll_directory(cuda_bin)
            except Exception:
                pass

def emit_event(event_type, **kwargs):
    payload = {"type": event_type, **kwargs}
    print(json.dumps(payload), flush=True)

def ensure_model_symlinks(cache_dir: Path) -> Path:
    try:
        cache_dir.mkdir(parents=True, exist_ok=True)
    except Exception:
        pass
    
    # Candidate locations for pre-downloaded Ultimate Vocal Remover models
    candidate_roots = [
        Path("/Applications/Ultimate Vocal Remover.app/Contents/Resources/models"),
    ]
    if sys.platform == "win32":
        local_app = os.environ.get("LOCALAPPDATA")
        if local_app:
            candidate_roots.append(Path(local_app) / "Programs" / "Ultimate Vocal Remover" / "models")
        for pf in ["ProgramFiles", "ProgramFiles(x86)"]:
            val = os.environ.get(pf)
            if val:
                candidate_roots.append(Path(val) / "Ultimate Vocal Remover" / "models")

    found_root = None
    for r in candidate_roots:
        if r.exists():
            found_root = r
            break

    if not found_root:
        emit_event("log", message=f"Notice: Local UVR application models not found; audio-separator will download required models as needed.")
        return cache_dir

    # Link/copy files recursively so audio-separator can resolve any model without network download
    for root, dirs, files in os.walk(found_root):
        for f in files:
            src = Path(root) / f
            dest = cache_dir / f
            if not dest.exists():
                linked = False
                try:
                    dest.hardlink_to(src)
                    linked = True
                except Exception:
                    pass
                if not linked:
                    try:
                        dest.symlink_to(src)
                        linked = True
                    except Exception:
                        pass
                if not linked:
                    try:
                        shutil.copy2(src, dest)
                    except Exception:
                        pass
    return cache_dir

def convert_wav_to_aac(wav_path: str, m4a_path: str, bitrate: int = 256000) -> bool:
    """Uses native macOS afconvert or cross-platform ffmpeg to encode AAC .m4a."""
    if sys.platform == "darwin" and os.path.exists("/usr/bin/afconvert"):
        cmd = [
            "/usr/bin/afconvert",
            "-f", "m4af",
            "-d", "aac",
            "-b", str(bitrate),
            wav_path,
            m4a_path
        ]
    else:
        bitrate_k = f"{int(bitrate / 1000)}k"
        cmd = [
            "ffmpeg",
            "-y",
            "-i", str(wav_path),
            "-c:a", "aac",
            "-b:a", bitrate_k,
            str(m4a_path)
        ]

    try:
        kwargs = {"capture_output": True, "text": True}
        if sys.platform == "win32" and hasattr(subprocess, "CREATE_NO_WINDOW"):
            kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW
        res = subprocess.run(cmd, **kwargs)
        if res.returncode != 0:
            emit_event("log", message=f"AAC encoding error: {res.stderr}")
            return False
        return os.path.exists(m4a_path)
    except Exception as e:
        emit_event("log", message=f"AAC encoding failed: {e}")
        return False

def clean_base_name(filename: str) -> str:
    base = Path(filename).stem
    # Strip any trailing (Vocals Ensemble) or (Vocals) if chaining lead/backups
    for suffix in [" (Vocals Ensemble)", " (Vocals)", " (Instrumental)", " (Iso Track)", "_Vocals", "_Instrumental"]:
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
            ensemble_algorithm="uvr_max_spec",
            log_level=20 # INFO
        )
        handler = UvrLogHandler()
        stdout_logger = logging.StreamHandler(sys.stdout)
        stdout_logger.setFormatter(logging.Formatter("%(asctime)s - %(levelname)s - %(name)s - %(message)s"))
        separator.logger.handlers = [handler, stdout_logger]
        separator.logger.propagate = False
        as_logger = logging.getLogger("audio_separator")
        as_logger.handlers = [handler, stdout_logger]
        as_logger.propagate = False

        emit_event("progress", percent=10, stage="Configuring Models & GPU Acceleration...")
        separator.load_model(models)

        emit_event("progress", percent=15, stage="Model 1/4: Kim Vocal 2 (MDX-Net)...")
        
        # Run separation
        output_stems = separator.separate(str(input_path))
        
        emit_event("progress", percent=95, stage="Ensemble Complete. Encoding pristine AAC stems...")

        if not output_stems:
            raise RuntimeError("Separation did not produce any output stems")

        vocals_wav = None
        inst_wav = None

        for stem in output_stems:
            stem_full = str(temp_dir_path / stem) if not os.path.isabs(stem) else stem
            stem_lower = stem.lower()
            if "(vocals)" in stem_lower or "_vocals" in stem_lower or "vocals" in stem_lower:
                vocals_wav = stem_full
            elif "(instrumental)" in stem_lower or "_instrumental" in stem_lower or "instrumental" in stem_lower or "inst" in stem_lower:
                inst_wav = stem_full

        if not vocals_wav and len(output_stems) > 0:
            vocals_wav = str(temp_dir_path / output_stems[0]) if not os.path.isabs(output_stems[0]) else output_stems[0]
        if not inst_wav and len(output_stems) > 1:
            inst_wav = str(temp_dir_path / output_stems[1]) if not os.path.isabs(output_stems[1]) else output_stems[1]

        # Output target setup
        output_dir.mkdir(parents=True, exist_ok=True)
        base_title = clean_base_name(input_path.name)

        results = []

        if vocals_wav and os.path.exists(vocals_wav):
            out_vocals_name = f"{base_title} (Vocals Ensemble).m4a"
            out_vocals_path = output_dir / out_vocals_name
            if convert_wav_to_aac(vocals_wav, str(out_vocals_path)):
                results.append({
                    "name": out_vocals_name,
                    "path": str(out_vocals_path),
                    "role": "vocals",
                    "fileType": "audio"
                })

        if inst_wav and os.path.exists(inst_wav):
            out_inst_name = f"{base_title} (Iso Track).m4a"
            out_inst_path = output_dir / out_inst_name
            if convert_wav_to_aac(inst_wav, str(out_inst_path)):
                results.append({
                    "name": out_inst_name,
                    "path": str(out_inst_path),
                    "role": "track",
                    "fileType": "audio"
                })

        if not results:
            raise RuntimeError("Failed to encode output stems to AAC")

        emit_event("progress", percent=100, stage="Done!")
        emit_event("complete", success=True, files=results)

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
        stdout_logger = logging.StreamHandler(sys.stdout)
        stdout_logger.setFormatter(logging.Formatter("%(asctime)s - %(levelname)s - %(name)s - %(message)s"))
        separator.logger.handlers = [handler, stdout_logger]
        separator.logger.propagate = False
        as_logger = logging.getLogger("audio_separator")
        as_logger.handlers = [handler, stdout_logger]
        as_logger.propagate = False

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
    parser.add_argument("--cache-dir", help="Model cache directory path")
    args = parser.parse_args()

    try:
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

        if args.cache_dir:
            cache_dir = Path(args.cache_dir).resolve()
        elif os.environ.get("TRACKHELM_MODELS_CACHE"):
            cache_dir = Path(os.environ["TRACKHELM_MODELS_CACHE"]).resolve()
        elif sys.platform == "win32" and os.environ.get("LOCALAPPDATA"):
            cache_dir = Path(os.environ["LOCALAPPDATA"]) / "TrackHelm" / "models_cache"
        else:
            project_root = Path(__file__).resolve().parent.parent
            cache_dir = project_root / ".models_cache"
        ensure_model_symlinks(cache_dir)

        if args.mode == "ensemble_vocals":
            run_ensemble_vocals(input_path, output_dir, cache_dir)
        elif args.mode == "lead_backups":
            run_lead_backups(input_path, output_dir, cache_dir)
    except Exception as e:
        emit_event("error", message=str(e))
        sys.exit(1)

if __name__ == "__main__":
    main()
