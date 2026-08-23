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

    // Clear background (dark black)
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = "#0a0a0a";
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
        ctx.fillStyle = "#99ccff"; // LCARS active blue
      } else {
        ctx.fillStyle = "#ff9900"; // LCARS standby gold
      }

      ctx.fillRect(x, y, Math.max(1, barWidth - 0.5), barHeight);
    }

    // Draw playhead vertical line
    const playheadX = progress * width;
    ctx.strokeStyle = "#ff3366"; // Vivid red playhead
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
  <!-- LCARS Design Header -->
  <header class="lcars-header">
    <div class="lcars-bar-left"></div>
    <div class="lcars-title-block">
      <span class="system-id">SYS-9472</span>
      <h1>TRACKHELM // REHEARSAL PLAYBACK</h1>
    </div>
    <div class="lcars-bar-right">
      <span class="status-indicator">{isPlaying ? "ACTIVE" : "STANDBY"}</span>
    </div>
  </header>

  <div class="content-grid">
    <!-- Sidebar Navigation Skeletons -->
    <aside class="sidebar">
      <button class="nav-btn active">WORKSTATION</button>
      <button class="nav-btn" on:click={handleLoad}>LOAD TRACK</button>
      <div class="lcars-deco-block lcars-pink"></div>
      <div class="lcars-deco-block lcars-purple"></div>
    </aside>

    <!-- Main Workspace Display -->
    <section class="main-display">
      
      <!-- Current Track Details Panel -->
      <div class="status-panel">
        <div class="meta-row">
          <div>TRACK: <span class="highlight">{fileName || "No File Loaded"}</span></div>
          <div>FORMAT: <span class="highlight">{fileName ? `${channels}ch @ ${sampleRate / 1000}kHz` : "None"}</span></div>
        </div>
        <div class="meta-row font-large">
          <div>PLAYHEAD: <span class="highlight">{formatTime(currentTime)}</span> / {formatTime(duration)}</div>
        </div>
      </div>

      <!-- Waveform Window -->
      <div class="waveform-container">
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
        <canvas
          bind:this={canvasElement}
          width="800"
          height="180"
          on:click={handleCanvasClick}
          class="waveform-canvas"
        ></canvas>
      </div>

      <!-- Transport Control Panel -->
      <div class="controls-panel">
        <div class="button-group">
          <button class="ctrl-btn lcars-blue" on:click={handleLoad}>
            OPEN
          </button>
          
          <button class="ctrl-btn lcars-gold" on:click={handlePlayPause}>
            {isPlaying ? "PAUSE" : "PLAY"}
          </button>

          <button class="ctrl-btn lcars-purple" on:click={handleStop}>
            STOP
          </button>
        </div>

        <div class="volume-slider-group">
          <span class="label">VOL: {Math.round(volume * 100)}%</span>
          <input
            type="range"
            min="0"
            max="1.5"
            step="0.05"
            value={volume}
            on:input={handleVolume}
            class="vol-slider"
          />
        </div>
      </div>
    </section>
  </div>
</main>

<style>
  :global(body) {
    background-color: #000000;
    color: #ff9900; /* LCARS gold */
    font-family: "Courier New", Courier, monospace;
    margin: 0;
    padding: 0;
  }

  .container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    padding: 10px;
    box-sizing: border-box;
  }

  .lcars-header {
    display: flex;
    align-items: flex-end;
    margin-bottom: 20px;
  }

  .lcars-bar-left {
    background-color: #99ccff;
    width: 60px;
    height: 40px;
    border-radius: 20px 0 0 20px;
  }

  .lcars-title-block {
    background-color: #ff9900;
    color: #000000;
    padding: 2px 20px;
    flex-grow: 1;
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-weight: bold;
    height: 36px;
  }

  .system-id {
    font-size: 0.8rem;
    color: #444444;
  }

  h1 {
    margin: 0;
    font-size: 1.2rem;
  }

  .lcars-bar-right {
    background-color: #ffcc00;
    width: 120px;
    height: 40px;
    border-radius: 0 20px 20px 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .status-indicator {
    color: #000000;
    font-weight: bold;
    font-size: 0.9rem;
  }

  .content-grid {
    display: flex;
    flex-grow: 1;
    gap: 15px;
  }

  .sidebar {
    width: 150px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .nav-btn {
    background-color: #cc6699;
    color: #000000;
    border: none;
    border-radius: 10px 0 0 10px;
    padding: 12px 15px;
    font-weight: bold;
    text-align: right;
    cursor: pointer;
    font-size: 0.9rem;
  }

  .nav-btn.active {
    background-color: #99ccff;
  }

  .lcars-deco-block {
    height: 40px;
    border-radius: 10px 0 0 10px;
  }

  .lcars-pink { background-color: #cc6699; }
  .lcars-purple { background-color: #9966cc; }

  .main-display {
    flex-grow: 1;
    border: 2px solid #ffcc00;
    border-radius: 0 15px 15px 15px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    background-color: #050505;
  }

  .status-panel {
    border-bottom: 2px solid #444444;
    padding-bottom: 15px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .meta-row {
    display: flex;
    justify-content: space-between;
    font-size: 1rem;
  }

  .font-large {
    font-size: 1.3rem;
    font-weight: bold;
  }

  .highlight {
    color: #99ccff;
  }

  .waveform-container {
    border: 2px solid #555555;
    background-color: #0a0a0a;
    border-radius: 10px;
    overflow: hidden;
    height: 180px;
  }

  .waveform-canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: pointer;
  }

  .controls-panel {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-top: 1px solid #444444;
    padding-top: 15px;
  }

  .button-group {
    display: flex;
    gap: 10px;
  }

  .ctrl-btn {
    border: none;
    border-radius: 15px;
    padding: 12px 30px;
    font-weight: bold;
    font-size: 1rem;
    cursor: pointer;
    color: #000000;
  }

  .lcars-blue { background-color: #99ccff; }
  .lcars-gold { background-color: #ff9900; }
  .lcars-purple { background-color: #9966cc; }

  .volume-slider-group {
    display: flex;
    align-items: center;
    gap: 15px;
  }

  .label {
    font-weight: bold;
    color: #ffcc00;
  }

  .vol-slider {
    -webkit-appearance: none;
    appearance: none;
    background: #444444;
    height: 10px;
    border-radius: 5px;
    outline: none;
  }

  .vol-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #ff9900;
    cursor: pointer;
  }
</style>
