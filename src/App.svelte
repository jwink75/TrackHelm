<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  let filePath = "";
  let fileName = "";
  let duration = 0;
  let currentTime = 0;
  let isPlaying = false;
  let volume = 1.0;
  let peaks: number[] = [];
  let progress = 0.0;
  let channels = 2;
  let sampleRate = 44100;

  let canvasElement: HTMLCanvasElement;
  let statusInterval: any;

  // Poll engine status periodically
  onMount(() => {
    statusInterval = setInterval(async () => {
      try {
        const status: any = await invoke("get_playback_status");
        isPlaying = status.is_playing;
        currentTime = status.current_time;
        duration = status.duration_seconds;
        progress = status.progress;
        
        if (peaks.length > 0) {
          drawWaveform();
        }
      } catch (err) {
        console.error("Failed to query playback status", err);
      }
    }, 50);

    return () => {
      clearInterval(statusInterval);
    };
  });

  async function handleLoad() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{
          name: "Audio Files",
          extensions: ["wav", "mp3", "flac", "m4a", "aiff", "ogg"]
        }]
      });

      if (selected && typeof selected === "string") {
        filePath = selected;
        fileName = filePath.split("/").pop() || filePath;
        
        // Call backend loader
        const metadata: any = await invoke("load_track", { path: filePath });
        duration = metadata.duration_seconds;
        sampleRate = metadata.sample_rate;
        channels = metadata.channels;
        peaks = metadata.peaks;

        // Draw waveform
        setTimeout(drawWaveform, 50);
      }
    } catch (err) {
      alert("Failed to load file: " + err);
    }
  }

  async function handlePlayPause() {
    if (isPlaying) {
      await invoke("pause");
    } else {
      await invoke("play");
    }
  }

  async function handleStop() {
    await invoke("stop");
    currentTime = 0;
    progress = 0;
    drawWaveform();
  }

  async function handleVolume(e: Event) {
    const target = e.target as HTMLInputElement;
    volume = parseFloat(target.value);
    await invoke("set_volume", { volume });
  }

  // Seek by clicking on the waveform canvas
  function handleCanvasClick(e: MouseEvent) {
    if (duration === 0) return;
    const rect = canvasElement.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const pct = clickX / rect.width;
    const targetSeconds = pct * duration;
    invoke("seek", { seconds: targetSeconds });
  }

  // Draw the waveform + current playhead on Canvas
  function drawWaveform() {
    if (!canvasElement || peaks.length === 0) return;
    const ctx = canvasElement.getContext("2d");
    if (!ctx) return;

    const width = canvasElement.width;
    const height = canvasElement.height;

    // Clear background (dark grey)
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = "#1e1e1e";
    ctx.fillRect(0, 0, width, height);

    const barWidth = width / peaks.length;
    const halfHeight = height / 2;

    // Draw samples
    for (let i = 0; i < peaks.length; i++) {
      const val = peaks[i];
      const barHeight = val * (height * 0.85); // pad slightly
      
      const x = i * barWidth;
      const y = halfHeight - barHeight / 2;

      // Color coding: completed (played) area vs unplayed area
      const isPastPlayhead = (i / peaks.length) < progress;
      if (isPastPlayhead) {
        ctx.fillStyle = "#00adb5"; // Cyan played area
      } else {
        ctx.fillStyle = "#555555"; // Muted grey unplayed area
      }

      ctx.fillRect(x, y, Math.max(1, barWidth - 0.5), barHeight);
    }

    // Draw playhead vertical line
    const playheadX = progress * width;
    ctx.strokeStyle = "#ff2e63"; // Sharp pink/red playhead
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(playheadX, 0);
    ctx.lineTo(playheadX, height);
    ctx.stroke();
  }

  // Helper to format time label (MM:SS)
  function formatTime(secs: number) {
    if (isNaN(secs) || secs < 0) return "00:00";
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  }
</script>

<main class="container">
  <!-- Clean, Modern Minimalist Header -->
  <header class="app-header">
    <div class="brand">
      <span class="logo-icon">▲</span>
      <h1>TrackHelm</h1>
      <span class="version-tag">Rehearsal Workspace</span>
    </div>
    <div class="status-badge" class:active={isPlaying}>
      {isPlaying ? "PLAYING" : "STOPPED"}
    </div>
  </header>

  <div class="workspace-layout">
    <!-- Main Panel -->
    <section class="main-panel">
      
      <!-- Track Meta Info & Time Readout -->
      <div class="display-card">
        <div class="track-info">
          <span class="meta-label">NOW LOADING</span>
          <h2>{fileName || "No Audio File Selected"}</h2>
          {#if fileName}
            <span class="meta-details">{channels} Channels • {sampleRate / 1000} kHz</span>
          {/if}
        </div>

        <div class="time-readout">
          <span class="current-time">{formatTime(currentTime)}</span>
          <span class="divider">/</span>
          <span class="total-time">{formatTime(duration)}</span>
        </div>
      </div>

      <!-- Waveform Window -->
      <div class="waveform-box">
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
        <canvas
          bind:this={canvasElement}
          width="850"
          height="160"
          on:click={handleCanvasClick}
          class="waveform-canvas"
        ></canvas>
      </div>

      <!-- Playback Actions Footer -->
      <div class="toolbar">
        <div class="playback-controls">
          <button class="btn btn-outline" on:click={handleLoad}>
            Open File
          </button>
          
          <button class="btn btn-primary" on:click={handlePlayPause}>
            {isPlaying ? "Pause" : "Play"}
          </button>

          <button class="btn btn-secondary" on:click={handleStop}>
            Stop
          </button>
        </div>

        <!-- Volume Control -->
        <div class="volume-control">
          <span class="volume-label">VOL</span>
          <input
            type="range"
            min="0"
            max="1.5"
            step="0.05"
            value={volume}
            on:input={handleVolume}
            class="volume-slider"
          />
          <span class="volume-pct">{Math.round(volume * 100)}%</span>
        </div>
      </div>
    </section>
  </div>
</main>

<style>
  :global(body) {
    background-color: #0f0f11;
    color: #e4e4e7;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    margin: 0;
    padding: 0;
    -webkit-user-select: none;
    user-select: none;
  }

  .container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    padding: 24px;
    box-sizing: border-box;
    max-width: 1000px;
    margin: 0 auto;
    justify-content: center;
  }

  .app-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 20px;
    border-bottom: 1px solid #27272a;
    padding-bottom: 16px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .logo-icon {
    color: #00adb5;
    font-size: 1.2rem;
  }

  h1 {
    margin: 0;
    font-size: 1.4rem;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: #ffffff;
  }

  .version-tag {
    font-size: 0.8rem;
    background-color: #27272a;
    color: #a1a1aa;
    padding: 2px 8px;
    border-radius: 4px;
    font-weight: 500;
  }

  .status-badge {
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 6px 12px;
    border-radius: 12px;
    background-color: #27272a;
    color: #a1a1aa;
    transition: all 0.2s ease;
  }

  .status-badge.active {
    background-color: rgba(0, 173, 181, 0.15);
    color: #00adb5;
  }

  .workspace-layout {
    display: flex;
    flex-direction: column;
    flex-grow: 1;
  }

  .main-panel {
    background-color: #18181b;
    border: 1px solid #27272a;
    border-radius: 12px;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    box-shadow: 0 4px 20px rgba(0,0,0,0.3);
  }

  .display-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #0f0f11;
    border: 1px solid #27272a;
    border-radius: 8px;
    padding: 20px;
  }

  .track-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .meta-label {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: #00adb5;
  }

  h2 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
    color: #ffffff;
    word-break: break-all;
  }

  .meta-details {
    font-size: 0.8rem;
    color: #71717a;
  }

  .time-readout {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-family: Menlo, Monaco, Consolas, "Courier New", monospace;
  }

  .current-time {
    font-size: 2.2rem;
    font-weight: 700;
    color: #ffffff;
  }

  .divider {
    font-size: 1.2rem;
    color: #3f3f46;
  }

  .total-time {
    font-size: 1.2rem;
    color: #71717a;
  }

  .waveform-box {
    border: 1px solid #27272a;
    border-radius: 8px;
    overflow: hidden;
    height: 160px;
    background-color: #1e1e1e;
  }

  .waveform-canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: pointer;
  }

  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-top: 1px solid #27272a;
    padding-top: 16px;
  }

  .playback-controls {
    display: flex;
    gap: 12px;
  }

  .btn {
    border: none;
    border-radius: 6px;
    padding: 10px 20px;
    font-weight: 600;
    font-size: 0.9rem;
    cursor: pointer;
    transition: background-color 0.2s ease;
  }

  .btn-outline {
    background-color: transparent;
    color: #e4e4e7;
    border: 1px solid #3f3f46;
  }

  .btn-outline:hover {
    background-color: #27272a;
  }

  .btn-primary {
    background-color: #00adb5;
    color: #ffffff;
  }

  .btn-primary:hover {
    background-color: #008f96;
  }

  .btn-secondary {
    background-color: #3f3f46;
    color: #e4e4e7;
  }

  .btn-secondary:hover {
    background-color: #52525b;
  }

  .volume-control {
    display: flex;
    align-items: center;
    gap: 12px;
    background-color: #0f0f11;
    border: 1px solid #27272a;
    padding: 6px 16px;
    border-radius: 20px;
  }

  .volume-label {
    font-size: 0.75rem;
    font-weight: 700;
    color: #71717a;
  }

  .volume-slider {
    -webkit-appearance: none;
    appearance: none;
    background: #27272a;
    height: 6px;
    border-radius: 3px;
    outline: none;
    width: 100px;
  }

  .volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #00adb5;
    cursor: pointer;
    transition: transform 0.1s ease;
  }

  .volume-slider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
  }

  .volume-pct {
    font-size: 0.8rem;
    font-family: monospace;
    color: #e4e4e7;
    width: 35px;
    text-align: right;
  }
</style>
