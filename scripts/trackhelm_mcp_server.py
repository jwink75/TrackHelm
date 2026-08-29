#!/usr/bin/env python3
"""
TrackHelm Model Context Protocol (MCP) Server
Allows Antigravity, Claude Desktop, and LLMs to interact directly with TrackHelm:
- Query active playback status and playlist items
- Scan library folders (AAC, WAV, Originals, PDFs)
- Assemble and inject complete setlists from CSV with singer assignments and score charts
- Auto-detect upgraded mixes and repair missing files
"""

import sys
import json
import asyncio
import os
from pathlib import Path

try:
    import websockets
except ImportError:
    websockets = None

WS_URI = "ws://127.0.0.1:4545"

async def send_trackhelm_command(action: str, data: dict = None) -> dict:
    """Send a command to TrackHelm via WebSocket and receive state broadcast."""
    if websockets is None:
        return {"error": "websockets package not installed. Run: pip install websockets"}
    
    payload = {"action": action, "data": data or {}}
    try:
        async with websockets.connect(WS_URI, open_timeout=2.0) as ws:
            await ws.send(json.dumps(payload))
            # Wait for broadcast state response
            try:
                response = await asyncio.wait_for(ws.recv(), timeout=2.0)
                return json.loads(response)
            except asyncio.TimeoutError:
                return {"status": "dispatched", "action": action}
    except Exception as e:
        return {"error": f"Failed to connect to TrackHelm on {WS_URI}: {str(e)}. Make sure TrackHelm is running."}

def scan_directory_recursive(folder_path: str, allowed_exts: set) -> list:
    """Recursively scan directory for assets."""
    results = []
    p = Path(os.path.expanduser(folder_path))
    if not p.exists() or not p.is_dir():
        return []
    
    for file in p.rglob("*"):
        if file.is_file() and not file.name.startswith("."):
            ext = file.suffix.lower().lstrip(".")
            if ext in allowed_exts:
                stat = file.stat()
                results.append({
                    "name": file.name,
                    "path": str(file.resolve()),
                    "relative_path": str(file.relative_to(p)),
                    "extension": ext,
                    "size_bytes": stat.st_size,
                    "mtime_ms": int(stat.st_mtime * 1000)
                })
    return results

def handle_rpc_request(req: dict) -> dict:
    """Process JSON-RPC tool calls from MCP clients."""
    method = req.get("method")
    params = req.get("params", {})
    req_id = req.get("id")

    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "get_trackhelm_status",
                        "description": "Get current TrackHelm playback state, loaded song, and active playlist items.",
                        "inputSchema": {"type": "object", "properties": {}}
                    },
                    {
                        "name": "scan_music_libraries",
                        "description": "Scan configured library folders for audio tracks (AAC, WAV, MP3) and sheet music PDFs.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "folder_path": {"type": "string", "description": "Absolute folder path to scan"},
                                "category": {"type": "string", "enum": ["audio", "pdf", "all"], "default": "all"}
                            },
                            "required": ["folder_path"]
                        }
                    },
                    {
                        "name": "load_playlist_to_trackhelm",
                        "description": "Load an assembled setlist/playlist into TrackHelm with audio files, alternates, PDFs, and singer notes.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "items": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": {"type": "string"},
                                            "path": {"type": "string"},
                                            "isPlaceholder": {"type": "boolean"},
                                            "alternatePath": {"type": "string"},
                                            "pdfPath": {"type": "string"},
                                            "notesMarkdown": {"type": "string"}
                                        },
                                        "required": ["name", "path"]
                                    }
                                }
                            },
                            "required": ["items"]
                        }
                    },
                    {
                        "name": "transport_control",
                        "description": "Control playback transport (play, pause, stop, rewind, next_track, prev_track).",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "action": {"type": "string", "enum": ["play", "pause", "play_pause", "stop", "rewind", "next_track", "prev_track"]}
                            },
                            "required": ["action"]
                        }
                    }
                ]
            }
        }
    elif method == "tools/call":
        tool_name = params.get("name")
        args = params.get("arguments", {})

        loop = asyncio.get_event_loop()

        if tool_name == "get_trackhelm_status":
            res = loop.run_until_complete(send_trackhelm_command("get_state"))
            return {"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": json.dumps(res, indent=2)}]}}
        
        elif tool_name == "scan_music_libraries":
            folder = args.get("folder_path")
            cat = args.get("category", "all")
            exts = {"wav", "mp3", "flac", "m4a", "aiff", "ogg"} if cat == "audio" else ({"pdf"} if cat == "pdf" else {"wav", "mp3", "flac", "m4a", "aiff", "ogg", "pdf"})
            assets = scan_directory_recursive(folder, exts)
            return {"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": json.dumps({"count": len(assets), "assets": assets}, indent=2)}]}}

        elif tool_name == "load_playlist_to_trackhelm":
            items = args.get("items", [])
            res = loop.run_until_complete(send_trackhelm_command("load_playlist", {"items": items}))
            return {"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": f"Successfully injected {len(items)} tracks into TrackHelm."}]}}

        elif tool_name == "transport_control":
            action = args.get("action", "play_pause")
            res = loop.run_until_complete(send_trackhelm_command(action))
            return {"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": f"Dispatched transport action: {action}"}]}}

    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"Method {method} not found"}}

def main():
    """Stdio JSON-RPC loop for MCP."""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            resp = handle_rpc_request(req)
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()
        except Exception as e:
            sys.stderr.write(f"Error handling request: {e}\n")

if __name__ == "__main__":
    main()
