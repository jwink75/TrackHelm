<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  // Active Playback State
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

  // Rehearsal Workstation State
  let speed = 1.0; // Placeholder for Milestone 2
  let pitch = 0;   // Placeholder for Milestone 2
  let compressorThreshold = -20; // Placeholder for Milestone 4
  let compressorMakeup = 0;      // Placeholder for Milestone 4
  let eqBass = 0;                // Placeholder for Milestone 3
  let eqTreble = 0;              // Placeholder for Milestone 3

  // Markers
  interface Marker {
    id: number;
    name: string;
    time: number;
  }
  let markers: Marker[] = [];
  let nextMarkerId = 1;

  // Playlist Browser (Mock list of files + currently loaded)
  let playlist = [
    { name: "Demo Song - Stereo.wav", path: "" },
    { name: "Rehearsal Take 1.mp3", path: "" }
  ];

  let mainCanvas: HTMLCanvasElement;
  let overviewCanvas: HTMLCanvasElement;
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
          drawMainWaveform();
          drawOverviewWaveform();
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
        await loadAudioPath(selected);
      }
    } catch (err) {
      alert("Failed to load file: " + err);
    }
  }

  async function loadAudioPath(path: string) {
    filePath = path;
    fileName = path.split("/").pop() || path;
    
    // Call backend loader
    const metadata: any = await invoke("load_track", { path: filePath });
    duration = metadata.duration_seconds;
    sampleRate = metadata.sample_rate;
    channels = metadata.channels;
    peaks = metadata.peaks;

    // Reset markers for new track
    markers = [];
    nextMarkerId = 1;

    // Add to playlist if not already there
    if (!playlist.some(item => item.path === path)) {
      playlist = [...playlist, { name: fileName, path }];
    }

    // Redraw canvases
    setTimeout(() => {
      drawMainWaveform();
      drawOverviewWaveform();
    }, 50);
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
    drawMainWaveform();
    drawOverviewWaveform();
  }

  async function handleVolume(e: Event) {
    const target = e.target as HTMLInputElement;
    volume = parseFloat(target.value);
    await invoke("set_volume", { volume });
  }

  function handleRewind() {
    invoke("seek", { seconds: 0 });
  }

  // Seek by clicking on the waveform canvas
  function handleMainWaveformClick(e: MouseEvent) {
    if (duration === 0) return;
    const rect = mainCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const pct = clickX / rect.width;
    const targetSeconds = pct * duration;
    invoke("seek", { seconds: targetSeconds });
  }

  function handleOverviewClick(e: MouseEvent) {
    if (duration === 0) return;
    const rect = overviewCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const pct = clickX / rect.width;
    const targetSeconds = pct * duration;
    invoke("seek", { seconds: targetSeconds });
  }

  // Markers operations
  function addMarker() {
    if (duration === 0) return;
    const newMarker: Marker = {
      id: nextMarkerId++,
      name: `Marker ${nextMarkerId - 1}`,
      time: currentTime
    };
    markers = [...markers, newMarker].sort((a, b) => a.time - b.time);
  }

  function deleteMarker(id: number) {
    markers = markers.filter(m => m.id !== id);
  }

  function seekToMarker(time: number) {
    invoke("seek", { seconds: time });
  }

  function jumpToPrevMarker() {
    if (markers.length === 0) return;
    // Find largest marker time smaller than current time - 0.5s
    const prev = [...markers]
      .reverse()
      .find(m => m.time < currentTime - 0.5);
    if (prev) {
      seekToMarker(prev.time);
    } else {
      invoke("seek", { seconds: 0 });
    }
  }

  function jumpToNextMarker() {
    if (markers.length === 0) return;
    // Find smallest marker time larger than current time
    const next = markers.find(m => m.time > currentTime + 0.1);
    if (next) {
      seekToMarker(next.time);
    }
  }

  // Draw the main scrolling waveform
  function drawMainWaveform() {
    if (!mainCanvas || peaks.length === 0) return;
    const ctx = mainCanvas.getContext("2d");
    if (!ctx) return;

    const width = mainCanvas.width;
    const height = mainCanvas.height;

    // Colored Backdrop (Anytune style - deep blue/slate gradient)
    ctx.clearRect(0, 0, width, height);
    const gradient = ctx.createLinearGradient(0, 0, 0, height);
    gradient.addColorStop(0, "#1a2c3a");
    gradient.addColorStop(1, "#111e28");
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, width, height);

    // Grid lines
    ctx.strokeStyle = "#1b384d";
    ctx.lineWidth = 1;
    for (let x = 50; x < width; x += 100) {
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
    }

    const barWidth = width / peaks.length;
    const halfHeight = height / 2;

    // Draw waveform (White peaks)
    ctx.fillStyle = "#ffffff";
    for (let i = 0; i < peaks.length; i++) {
      const val = peaks[i];
      const barHeight = val * (height * 0.8);
      const x = i * barWidth;
      const y = halfHeight - barHeight / 2;
      ctx.fillRect(x, y, Math.max(1, barWidth - 0.5), barHeight);
    }

    // Draw markers on waveform
    ctx.lineWidth = 1;
    for (const marker of markers) {
      const markerPct = marker.time / duration;
      const markerX = markerPct * width;
      ctx.strokeStyle = "#ff9500"; // Dorico Amber for markers
      ctx.beginPath();
      ctx.moveTo(markerX, 0);
      ctx.lineTo(markerX, height);
      ctx.stroke();
      
      // Marker label
      ctx.fillStyle = "#ff9500";
      ctx.font = "9px sans-serif";
      ctx.fillText(marker.name, markerX + 4, 12);
    }

    // Draw playhead vertical line (Dorico highlight cyan)
    const playheadX = progress * width;
    ctx.strokeStyle = "#3b99fc"; 
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(playheadX, 0);
    ctx.lineTo(playheadX, height);
    ctx.stroke();
  }

  // Draw the overview waveform
  function drawOverviewWaveform() {
    if (!overviewCanvas || peaks.length === 0) return;
    const ctx = overviewCanvas.getContext("2d");
    if (!ctx) return;

    const width = overviewCanvas.width;
    const height = overviewCanvas.height;

    // Dark grey backdrop
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = "#1e1e1e";
    ctx.fillRect(0, 0, width, height);

    // Draw simplified peaks in muted blue
    const barWidth = width / peaks.length;
    const halfHeight = height / 2;
    ctx.fillStyle = "#4a6572";
    for (let i = 0; i < peaks.length; i += 2) {
      const val = peaks[i];
      const barHeight = val * (height * 0.7);
      const x = i * barWidth;
      const y = halfHeight - barHeight / 2;
      ctx.fillRect(x, y, Math.max(1, barWidth * 2 - 0.5), barHeight);
    }

    // Draw current window overlay (cyan box representing playhead position)
    const playheadX = progress * width;
    ctx.fillStyle = "rgba(59, 153, 252, 0.3)";
    ctx.fillRect(0, 0, playheadX, height);
    
    ctx.strokeStyle = "#3b99fc";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(playheadX, 0);
    ctx.lineTo(playheadX, height);
    ctx.stroke();
  }

  // Format Helper (MM:SS.hh)
  function formatTime(secs: number) {
    if (isNaN(secs) || secs < 0) return "00:00.00";
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    const ms = Math.floor((secs % 1) * 100);
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}.${ms.toString().padStart(2, "0")}`;
  }
</script>

<main class="app-container">
  <!-- Top bar (OS Stuff / Title) -->
  <header class="app-header">
    <div class="header-left">
      <span class="logo">TrackHelm</span>
      <span class="file-path">{filePath || "No File Loaded"}</span>
    </div>
    <div class="header-right">
      <span class="status-indicator" class:playing={isPlaying}>
        {isPlaying ? "PLAYING" : "STANDBY"}
      </span>
    </div>
  </header>

  <div class="workspace-grid">
    
    <!-- LEFT SIDEBAR: Browser / Playlist -->
    <aside class="sidebar-left">
      <div class="panel-header">PLAYLIST / BROWSER</div>
      
      <div class="playlist-box">
        {#each playlist as item}
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="playlist-item" 
            class:active={fileName === item.name}
            on:click={() => item.path && loadAudioPath(item.path)}
          >
            <span class="item-icon">🎵</span>
            <span class="item-name">{item.name}</span>
          </div>
        {/each}
      </div>

      <div class="selection-info">
        <div class="panel-header">SELECTION INFO</div>
        <div class="info-row">
          <span class="label">Total Time:</span>
          <span class="val">{formatTime(duration)}</span>
        </div>
        <div class="info-row">
          <span class="label">Elapsed:</span>
          <span class="val">{formatTime(currentTime)}</span>
        </div>
        <div class="info-row">
          <span class="label">Remaining:</span>
          <span class="val">{formatTime(Math.max(0, duration - currentTime))}</span>
        </div>
      </div>

      <button class="action-btn file-btn" on:click={handleLoad}>
        Load Custom File
      </button>
    </aside>

    <!-- CENTER AREA: Waveforms & Transport -->
    <section class="center-content">
      
      <!-- Current File info header -->
      <div class="track-header">
        <div class="track-title-info">
          <h2>{fileName || "Load a track to begin"}</h2>
          {#if fileName}
            <span class="track-spec">{channels} Channels • {sampleRate / 1000} kHz</span>
          {/if}
        </div>
        <div class="time-readout">
          <span class="time-large">{formatTime(currentTime)}</span>
          <span class="time-sep">/</span>
          <span class="time-total">{formatTime(duration)}</span>
        </div>
      </div>

      <!-- Overview Waveform -->
      <div class="waveform-overview-container">
        <div class="waveform-label">OVERVIEW</div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
        <canvas 
          bind:this={overviewCanvas} 
          width="800" 
          height="45" 
          on:click={handleOverviewClick}
          class="overview-canvas"
        ></canvas>
      </div>

      <!-- Main Waveform Box (Anytune-style White-on-Blue Backdrop) -->
      <div class="main-waveform-container">
        <div class="waveform-label">MAIN WAVEFORM</div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
        <canvas 
          bind:this={mainCanvas} 
          width="800" 
          height="240" 
          on:click={handleMainWaveformClick}
          class="main-canvas"
        ></canvas>
      </div>

      <!-- Bottom controls bar (Transport & Loop controls) -->
      <div class="controls-row">
        <!-- Transport controls -->
        <div class="control-group">
          <div class="group-label">PLAYBACK</div>
          <div class="btn-row">
            <button class="control-btn" on:click={handleRewind} title="Rewind to start">⏮</button>
            <button class="control-btn" on:click={jumpToPrevMarker} title="Previous Marker">⏪</button>
            <button class="control-btn play-btn" on:click={handlePlayPause}>
              {isPlaying ? "⏸ PAUSE" : "▶ PLAY"}
            </button>
            <button class="control-btn" on:click={handleStop} title="Stop">⏹</button>
            <button class="control-btn" on:click={jumpToNextMarker} title="Next Marker">⏩</button>
          </div>
        </div>

        <!-- Looping and markers controls -->
        <div class="control-group">
          <div class="group-label">MARKERS / LOOPING</div>
          <div class="btn-row">
            <button class="control-btn accent-btn" on:click={addMarker}>+ Add Marker</button>
            <button class="control-btn disabled-btn" title="Loop start (Milestone 5)">[ Loop</button>
            <button class="control-btn disabled-btn" title="Loop end (Milestone 5)">Loop ]</button>
          </div>
        </div>
      </div>
    </section>

    <!-- RIGHT SIDEBAR: Markers, FX, Stretch Controls (Dorico Theme) -->
    <aside class="sidebar-right">
      
      <!-- Project and Links -->
      <div class="panel-section">
        <div class="panel-header">PROJECT SETUP</div>
        <div class="linked-files">
          <span class="placeholder-text">Alternate Track: None (Milestone 7)</span>
        </div>
      </div>

      <!-- Markers/Regions List -->
      <div class="panel-section markers-section">
        <div class="panel-header">MARKERS & REGIONS</div>
        <div class="markers-list">
          {#if markers.length === 0}
            <div class="placeholder-text">No markers set. Click "+ Add Marker" during playback.</div>
          {:else}
            {#each markers as marker}
              <div class="marker-item">
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <!-- svelte-ignore a11y-no-static-element-interactions -->
                <span class="marker-name-btn" on:click={() => seekToMarker(marker.time)}>
                  📍 {marker.name} ({formatTime(marker.time)})
                </span>
                <button class="delete-marker-btn" on:click={() => deleteMarker(marker.id)}>×</button>
              </div>
            {/each}
          {/if}
        </div>
      </div>

      <!-- Effects Rack & Stretch (Sliders) -->
      <div class="panel-section dsp-section">
        <div class="panel-header">EFFECTS MODULES</div>

        <!-- Time & Pitch Stretch (Milestone 2) -->
        <div class="dsp-control">
          <div class="dsp-label-row">
            <span class="dsp-title">SPEED (Tempo)</span>
            <span class="dsp-value">{speed.toFixed(2)}x</span>
          </div>
          <input type="range" min="0.5" max="2.0" step="0.05" bind:value={speed} class="dsp-slider" />
        </div>

        <div class="dsp-control">
          <div class="dsp-label-row">
            <span class="dsp-title">PITCH SHIFT</span>
            <span class="dsp-value">{pitch > 0 ? "+" : ""}{pitch} semitones</span>
          </div>
          <input type="range" min="-12" max="12" step="1" bind:value={pitch} class="dsp-slider" />
        </div>

        <!-- Gain / Master Fader (Functional Volume) -->
        <div class="dsp-control active-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">MASTER GAIN</span>
            <span class="dsp-value">{Math.round(volume * 100)}%</span>
          </div>
          <input 
            type="range" 
            min="0" 
            max="1.5" 
            step="0.05" 
            value={volume} 
            on:input={handleVolume}
            class="dsp-slider" 
          />
        </div>

        <!-- Compressor Placeholders (Milestone 4) -->
        <div class="dsp-divider">COMPRESSOR</div>
        <div class="dsp-control placeholder-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">Threshold</span>
            <span class="dsp-value">{compressorThreshold} dB</span>
          </div>
          <input type="range" min="-60" max="0" step="1" bind:value={compressorThreshold} class="dsp-slider disabled-slider" disabled />
        </div>
        <div class="dsp-control placeholder-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">Makeup Gain</span>
            <span class="dsp-value">+{compressorMakeup} dB</span>
          </div>
          <input type="range" min="0" max="24" step="1" bind:value={compressorMakeup} class="dsp-slider disabled-slider" disabled />
        </div>

        <!-- EQ Placeholders (Milestone 3) -->
        <div class="dsp-divider">EQUALIZER (EQ)</div>
        <div class="dsp-control placeholder-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">Bass</span>
            <span class="dsp-value">{eqBass > 0 ? "+" : ""}{eqBass} dB</span>
          </div>
          <input type="range" min="-12" max="12" step="1" bind:value={eqBass} class="dsp-slider disabled-slider" disabled />
        </div>
        <div class="dsp-control placeholder-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">Treble</span>
            <span class="dsp-value">{eqTreble > 0 ? "+" : ""}{eqTreble} dB</span>
          </div>
          <input type="range" min="-12" max="12" step="1" bind:value={eqTreble} class="dsp-slider disabled-slider" disabled />
        </div>

      </div>
    </aside>

  </div>
</main>

<style>
  /* Color Scheme: Dorico (Slate grey / dark charcoal) */
  :global(body) {
    background-color: #1e1e1e; /* Dorico background dark slate */
    color: #d1d1d1; /* Light grey text */
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    margin: 0;
    padding: 0;
    -webkit-user-select: none;
    user-select: none;
    overflow: hidden;
  }

  .app-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    background-color: #181818;
  }

  /* Header Bar styling */
  .app-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #2d2d2d; /* Dorico dark grey toolbar */
    border-bottom: 1px solid #3c3c3c;
    padding: 8px 16px;
    height: 40px;
    box-sizing: border-box;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .logo {
    font-weight: 700;
    font-size: 1.1rem;
    letter-spacing: -0.02em;
    color: #3b99fc; /* Dorico active light blue */
  }

  .file-path {
    font-family: monospace;
    font-size: 0.75rem;
    color: #8e8e8e;
  }

  .status-indicator {
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 3px 8px;
    border-radius: 3px;
    background-color: #222222;
    color: #8e8e8e;
    border: 1px solid #3c3c3c;
  }

  .status-indicator.playing {
    background-color: rgba(59, 153, 252, 0.15);
    color: #3b99fc;
    border-color: rgba(59, 153, 252, 0.4);
  }

  /* Grid layout spanning 3-column workspaces */
  .workspace-grid {
    display: grid;
    grid-template-columns: 240px 1fr 280px;
    flex-grow: 1;
    overflow: hidden;
    height: calc(100vh - 40px);
  }

  /* Shared Panels Styling */
  .panel-header {
    background-color: #2b2b2b;
    border-bottom: 1px solid #3c3c3c;
    padding: 6px 12px;
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #a5a5a5;
  }

  /* Left Sidebar styles */
  .sidebar-left {
    background-color: #252526; /* Dorico sidebar charcoal */
    border-right: 1px solid #3c3c3c;
    display: flex;
    flex-direction: column;
  }

  .playlist-box {
    flex-grow: 1;
    overflow-y: auto;
    padding: 8px 0;
  }

  .playlist-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    font-size: 0.85rem;
    cursor: pointer;
    border-bottom: 1px solid #2d2d2d;
    transition: background-color 0.15s ease;
  }

  .playlist-item:hover {
    background-color: #2e2e2f;
  }

  .playlist-item.active {
    background-color: #333d46;
    color: #ffffff;
    border-left: 3px solid #3b99fc;
  }

  .item-icon {
    font-size: 0.9rem;
  }

  .item-name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .selection-info {
    border-top: 1px solid #3c3c3c;
    background-color: #202021;
    padding-bottom: 8px;
  }

  .info-row {
    display: flex;
    justify-content: space-between;
    padding: 6px 16px;
    font-size: 0.8rem;
  }

  .info-row .label {
    color: #8e8e8e;
  }

  .info-row .val {
    font-family: monospace;
    font-weight: 600;
    color: #ffffff;
  }

  .action-btn {
    border: none;
    padding: 10px;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
    width: 100%;
    box-sizing: border-box;
    text-align: center;
  }

  .file-btn {
    background-color: #3b99fc;
    color: #ffffff;
  }

  .file-btn:hover {
    background-color: #258bf5;
  }

  /* Center Workspace area styling */
  .center-content {
    background-color: #1e1e1e;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
  }

  .track-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid #2d2d2d;
    padding-bottom: 12px;
  }

  h2 {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 600;
    color: #ffffff;
  }

  .track-spec {
    font-size: 0.75rem;
    color: #8e8e8e;
  }

  .time-readout {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-family: Menlo, Monaco, Consolas, monospace;
    background-color: #141414;
    padding: 8px 16px;
    border-radius: 6px;
    border: 1px solid #2d2d2d;
  }

  .time-large {
    font-size: 2rem;
    font-weight: 700;
    color: #ffffff;
  }

  .time-sep {
    color: #444444;
  }

  .time-total {
    font-size: 1.1rem;
    color: #8e8e8e;
  }

  /* Waveform labeling and grids */
  .waveform-label {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #8e8e8e;
    margin-bottom: 4px;
  }

  .waveform-overview-container {
    background-color: #1a1a1a;
    border: 1px solid #3c3c3c;
    border-radius: 4px;
    overflow: hidden;
  }

  .overview-canvas {
    display: block;
    width: 100%;
    height: 45px;
    cursor: pointer;
  }

  /* Main Waveform Box (Anytune style white on blue-slate) */
  .main-waveform-container {
    background-color: #122a3a; /* Anytune style blue */
    border: 1px solid #1b384d;
    border-radius: 4px;
    overflow: hidden;
  }

  .main-canvas {
    display: block;
    width: 100%;
    height: 240px;
    cursor: pointer;
  }

  /* Controls row */
  .controls-row {
    display: flex;
    gap: 20px;
    margin-top: 10px;
  }

  .control-group {
    background-color: #252526;
    border: 1px solid #3c3c3c;
    border-radius: 6px;
    padding: 10px 16px;
    flex-grow: 1;
  }

  .group-label {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #8e8e8e;
    margin-bottom: 8px;
  }

  .btn-row {
    display: flex;
    gap: 8px;
  }

  .control-btn {
    background-color: #333333;
    color: #d1d1d1;
    border: 1px solid #444444;
    padding: 8px 16px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.9rem;
    font-weight: 600;
    transition: all 0.15s ease;
  }

  .control-btn:hover:not(.disabled-btn) {
    background-color: #444444;
    border-color: #555555;
    color: #ffffff;
  }

  .play-btn {
    background-color: #3b99fc;
    color: #ffffff;
    border-color: #258bf5;
    flex-grow: 1;
    font-size: 0.95rem;
  }

  .play-btn:hover {
    background-color: #258bf5;
  }

  .accent-btn {
    background-color: #ff9500; /* Dorico Amber */
    color: #000000;
    border-color: #e08300;
  }

  .accent-btn:hover {
    background-color: #ffaa33;
  }

  .disabled-btn {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Right Sidebar styles */
  .sidebar-right {
    background-color: #252526; /* Dorico panels */
    border-left: 1px solid #3c3c3c;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .panel-section {
    border-bottom: 1px solid #3c3c3c;
    display: flex;
    flex-direction: column;
  }

  .linked-files {
    padding: 12px 16px;
  }

  .placeholder-text {
    font-size: 0.75rem;
    color: #717171;
    font-style: italic;
    line-height: 1.4;
  }

  /* Markers list panel */
  .markers-section {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    max-height: 280px;
  }

  .markers-list {
    flex-grow: 1;
    overflow-y: auto;
    padding: 8px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .marker-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #2d2d2d;
    border: 1px solid #3c3c3c;
    padding: 6px 10px;
    border-radius: 4px;
    font-size: 0.8rem;
  }

  .marker-name-btn {
    cursor: pointer;
    font-weight: 600;
    color: #ff9500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 200px;
  }

  .marker-name-btn:hover {
    text-decoration: underline;
  }

  .delete-marker-btn {
    background: transparent;
    border: none;
    color: #8e8e8e;
    cursor: pointer;
    font-size: 1.1rem;
    font-weight: bold;
    padding: 0 4px;
  }

  .delete-marker-btn:hover {
    color: #ff453a;
  }

  /* DSP Effects Rack panel */
  .dsp-section {
    padding-bottom: 16px;
  }

  .dsp-control {
    padding: 10px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .dsp-divider {
    background-color: #2b2b2b;
    border-top: 1px solid #3c3c3c;
    border-bottom: 1px solid #3c3c3c;
    padding: 3px 16px;
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #8e8e8e;
    margin-top: 12px;
  }

  .dsp-label-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.75rem;
    font-weight: 700;
  }

  .dsp-title {
    color: #d1d1d1;
  }

  .dsp-value {
    font-family: monospace;
    color: #3b99fc;
  }

  .active-dsp .dsp-value {
    color: #3b99fc;
  }

  .placeholder-dsp .dsp-title {
    color: #717171;
  }

  .placeholder-dsp .dsp-value {
    color: #717171;
  }

  /* Sliders customization */
  .dsp-slider {
    -webkit-appearance: none;
    appearance: none;
    background: #3c3c3c;
    height: 4px;
    border-radius: 2px;
    outline: none;
  }

  .dsp-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #3b99fc;
    cursor: pointer;
  }

  .dsp-slider::-webkit-slider-thumb:hover {
    background: #5faeff;
  }

  /* Disabled inputs for future features */
  .disabled-slider::-webkit-slider-thumb {
    background: #555555;
    cursor: not-allowed;
  }
</style>
