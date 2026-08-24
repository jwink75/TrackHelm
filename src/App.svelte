<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";

  // Active Playback State (for the backend engine)
  let filePath = "";
  let fileName = "";
  let duration = 0;
  let currentTime = 0;
  let isPlaying = false;
  let volumeLinear = 1.0;
  let progress = 0.0;
  let channels = 2;
  let sampleRate = 44100;

  // Track Models
  interface ChannelPeaks {
    min: number[];
    max: number[];
  }

  interface Track {
    name: string;
    path: string;
    duration: number;
    sampleRate: number;
    channels: number;
    overviewPeaks: number[];
    channelPeaks: ChannelPeaks[];
  }

  let mainTrack: Track | null = null;
  let alternateTrack: Track | null = null;
  let activeTrackMode: "main" | "alternate" = "main";

  // Left Panel tabs
  let activeTab: "browser" | "playlist" = "browser";

  // Rehearsal Workstation Knob State (Double-click resets)
  let speed = 1.0;                  // range 0.25 - 4.0 (25% - 400%)
  let pitch = 0;                    // range -24 - +24 semitones
  let eqBass = 0;                   // range -12 - +12 dB
  let eqTreble = 0;                 // range -12 - +12 dB
  let compressorThreshold = -20;    // range -60 - 0 dB
  let compressorRatio = 2.0;        // range 1.0 - 20.0 (Ratio)
  let compressorMakeup = 0;         // range 0 - 24 dB
  
  // Slider State (Dbl-click resets)
  let dbVolume = 0.0;               // range -60 - +12 dB
  let zoom = 1.0;                   // range 1.0 - 800.0
  $: target15sZoom = duration > 0 ? Math.max(1.0, duration / 15.0) : 1.0;

  // Zoom View State (Stereo Channels)
  interface VisibleChannelData {
    isSampleLevel: boolean;
    rawSamples: number[];
    min: number[];
    max: number[];
  }
  let visibleChannels: VisibleChannelData[] = [];

  // Markers
  interface Marker {
    id: number;
    name: string;
    time: number;
  }
  let markers: Marker[] = [];
  let nextMarkerId = 1;

  // OS Folder Browser State
  let currentPath = "";
  let parentPath: string | null = null;
  let browserEntries: any[] = [];
  let cloudFolders: PlaylistItem[] = [];
  
  // Search & Type to Jump State
  let searchQuery = "";
  $: filteredEntries = browserEntries.filter(entry => 
    entry.name.toLowerCase().includes(searchQuery.toLowerCase())
  );
  $: { if (currentPath) searchQuery = ""; } // Clear search on folder change

  let typeToJumpBuffer = "";
  let typeToJumpTimeout: any = null;
  let jumpDebounceTimeout: any = null;
  
  // Browser Multi-selection
  let selectedFilePaths = new Set<string>();
  let lastSelectedEntry: { name: string; path: string } | null = null;

  // Custom Context Menu State
  let showContextMenu = false;
  let contextMenuX = 0;
  let contextMenuY = 0;
  let contextMenuTargetFile: { name: string; path: string } | null = null;

  // Playlist State (saved in localStorage)
  interface PlaylistItem {
    name: string;
    path: string;
  }
  let playlistItems: PlaylistItem[] = [];

  // Project Setup: File Associations (saved in localStorage)
  let pdfChartPath = "";
  let pdfChartName = "";
  let associatedVersions: PlaylistItem[] = [];

  // Canvas elements
  let mainCanvas: HTMLCanvasElement;
  let overviewCanvas: HTMLCanvasElement;
  let altOverviewCanvas: HTMLCanvasElement;
  let centerContentElement: HTMLDivElement;
  
  let statusInterval: any;
  let resizeObserver: ResizeObserver;

  // Knob Drag Interaction State
  let activeKnob: {
    id: string;
    startY: number;
    startX: number;
    startVal: number;
    min: number;
    max: number;
    step: number;
    setValue: (val: number) => void;
  } | null = null;

  // Helper to map dB to Linear Amplitude
  function dbToLinear(db: number) {
    if (db <= -59.5) return 0.0; // Mute at bottom
    return Math.pow(10, db / 20);
  }

  onMount(() => {
    // Initial folder load
    const lastDir = localStorage.getItem("th_last_dir");
    loadBrowser(lastDir);

    // Fetch detected cloud folders dynamically
    invoke("get_cloud_folders")
      .then((folders: any) => {
        cloudFolders = folders;
      })
      .catch(err => {
        console.error("Failed to load cloud folders:", err);
      });

    // Restore Playlists and Project setup from LocalStorage
    const savedPlaylist = localStorage.getItem("th_playlist");
    if (savedPlaylist) playlistItems = JSON.parse(savedPlaylist);

    const savedPdf = localStorage.getItem("th_pdf_path");
    if (savedPdf) {
      pdfChartPath = savedPdf;
      pdfChartName = pdfChartPath.split("/").pop() || pdfChartPath;
    }

    const savedVersions = localStorage.getItem("th_associated_versions");
    if (savedVersions) associatedVersions = JSON.parse(savedVersions);

    // Restore last loaded tracks and active state
    (async () => {
      const lastMainPath = localStorage.getItem("th_last_main_track_path");
      const lastAltPath = localStorage.getItem("th_last_alt_track_path");
      const lastActiveMode = localStorage.getItem("th_last_active_track_mode") as "main" | "alternate" | null;

      try {
        if (lastMainPath) {
          await loadAudioPath(lastMainPath, "main");
        }
        if (lastAltPath) {
          await loadAudioPath(lastAltPath, "alternate");
        }
        if (lastActiveMode && lastActiveMode !== activeTrackMode) {
          await toggleActiveTrack(lastActiveMode);
        }
      } catch (err) {
        console.error("Failed to restore last loaded tracks:", err);
      }
    })();

    statusInterval = setInterval(async () => {
      try {
        const status: any = await invoke("get_playback_status");
        isPlaying = status.is_playing;
        currentTime = status.current_time;
        duration = status.duration_seconds;
        progress = status.progress;
        
        await updateVisiblePeaks();

        drawMainWaveform();
        drawOverviewWaveform();
        drawAltOverviewWaveform();
      } catch (err) {
        console.error("Failed to query playback status", err);
      }
    }, 50);

    // Resize observer for canvases
    if (centerContentElement) {
      resizeObserver = new ResizeObserver(() => {
        drawMainWaveform();
        drawOverviewWaveform();
        drawAltOverviewWaveform();
      });
      resizeObserver.observe(centerContentElement);
    }

    // Listen to native window file drops (Tauri drag & drop)
    const unlistenDragDrop = listen("tauri://drag-drop", (event: any) => {
      const paths = event.payload.paths;
      if (paths && paths.length > 0) {
        const audioPath = paths.find((p: string) => {
          const lower = p.toLowerCase();
          return lower.endsWith(".wav") || lower.endsWith(".mp3") || lower.endsWith(".flac") || lower.endsWith(".m4a") || lower.endsWith(".aiff") || lower.endsWith(".ogg");
        });
        if (audioPath) {
          loadAudioPath(audioPath, "main");
        }
      }
    });

    // Close context menu on window click
    const closeMenu = () => { showContextMenu = false; };
    window.addEventListener("click", closeMenu);

    return () => {
      clearInterval(statusInterval);
      if (resizeObserver) resizeObserver.disconnect();
      unlistenDragDrop.then(fn => fn());
      window.removeEventListener("click", closeMenu);
    };
  });

  // OS Folder Browser Loader
  async function loadBrowser(path: string | null) {
    try {
      const contents: any = await invoke("read_dir", { path });
      currentPath = contents.current_path;
      parentPath = contents.parent_path;
      browserEntries = contents.entries;
      selectedFilePaths.clear();
      lastSelectedEntry = null;
      localStorage.setItem("th_last_dir", currentPath);
    } catch (err) {
      alert("Failed to read directory: " + err);
    }
  }

  // Load a file into main or alternate track slots
  async function loadAudioPath(path: string, target: "main" | "alternate") {
    try {
      const metadata: any = await invoke("load_track", { path });
      const track: Track = {
        name: path.split("/").pop() || path,
        path: path,
        duration: metadata.duration_seconds,
        sampleRate: metadata.sample_rate,
        channels: metadata.channels,
        overviewPeaks: metadata.overview_peaks || [],
        channelPeaks: metadata.channel_peaks || []
      };

      if (target === "main") {
        mainTrack = track;
        localStorage.setItem("th_last_main_track_path", path);
      } else {
        alternateTrack = track;
        localStorage.setItem("th_last_alt_track_path", path);
      }

      filePath = path;
      fileName = track.name;
      duration = track.duration;
      sampleRate = track.sampleRate;
      channels = track.channels;
      activeTrackMode = target;
      localStorage.setItem("th_last_active_track_mode", target);

      // Default zoom: 15 seconds rehearsal chunk view
      if (duration > 0) {
        zoom = Math.max(1.0, duration / 15.0);
      }

      // Reset markers for new track
      markers = [];
      nextMarkerId = 1;

      setTimeout(() => {
        updateVisiblePeaks();
      }, 50);
    } catch (err) {
      alert("Failed to load file: " + err);
    }
  }

  // Toggle active track between Main and Alternate
  async function toggleActiveTrack(target: "main" | "alternate") {
    if (target === "main" && mainTrack) {
      activeTrackMode = "main";
      filePath = mainTrack.path;
      fileName = mainTrack.name;
      duration = mainTrack.duration;
      sampleRate = mainTrack.sampleRate;
      channels = mainTrack.channels;
      await invoke("load_track", { path: mainTrack.path });
    } else if (target === "alternate" && alternateTrack) {
      activeTrackMode = "alternate";
      filePath = alternateTrack.path;
      fileName = alternateTrack.name;
      duration = alternateTrack.duration;
      sampleRate = alternateTrack.sampleRate;
      channels = alternateTrack.channels;
      await invoke("load_track", { path: alternateTrack.path });
    }
    localStorage.setItem("th_last_active_track_mode", target);
    updateVisiblePeaks();
  }

  // Type-To-Jump Keyboard Listener
  function handleBrowserKeydown(e: KeyboardEvent) {
    // Disable if focused inside an input element (e.g. search box)
    const activeEl = document.activeElement;
    if (activeEl && (activeEl.tagName === "INPUT" || activeEl.tagName === "TEXTAREA")) {
      return;
    }

    // Capture standard printable characters (length 1) without Cmd/Ctrl modifiers
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      typeToJumpBuffer += e.key;

      // Clear existing timeouts
      clearTimeout(typeToJumpTimeout);
      typeToJumpTimeout = setTimeout(() => {
        typeToJumpBuffer = "";
      }, 1000); // Clear buffer after 1s of inactivity

      clearTimeout(jumpDebounceTimeout);
      jumpDebounceTimeout = setTimeout(() => {
        performTypeToJump();
      }, 300); // Debounce actual jump scroll by 300ms to avoid aggressive cursor twitching
    }
  }

  function performTypeToJump() {
    if (!typeToJumpBuffer) return;
    
    // Find first matching directory or file prefix in current filtered list
    const match = filteredEntries.find(entry => 
      entry.name.toLowerCase().startsWith(typeToJumpBuffer.toLowerCase())
    );

    if (match) {
      selectedFilePaths.clear();
      selectedFilePaths.add(match.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: match.name, path: match.path };

      // Scroll the newly active browser element into view smoothly
      setTimeout(() => {
        const el = document.querySelector(".browser-item.active");
        if (el) {
          el.scrollIntoView({ block: "nearest", behavior: "smooth" });
        }
      }, 50);
    }
  }

  // Handle Multi-select File Clicks
  function handleFileClick(e: MouseEvent, entry: any) {
    if (entry.is_dir) return;

    if (e.metaKey || e.ctrlKey) {
      // Toggle selection
      if (selectedFilePaths.has(entry.path)) {
        selectedFilePaths.delete(entry.path);
        selectedFilePaths = selectedFilePaths; // Trigger reactivity
      } else {
        selectedFilePaths.add(entry.path);
        selectedFilePaths = selectedFilePaths;
      }
      lastSelectedEntry = { name: entry.name, path: entry.path };
    } else {
      // Regular click replaces selection
      selectedFilePaths.clear();
      selectedFilePaths.add(entry.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: entry.name, path: entry.path };
    }
  }

  // Context Menu handlers
  function handleContextMenu(e: MouseEvent, entry: any) {
    if (entry.is_dir) return;
    e.preventDefault();
    e.stopPropagation();

    // If right-clicked item is not selected, select it exclusively
    if (!selectedFilePaths.has(entry.path)) {
      selectedFilePaths.clear();
      selectedFilePaths.add(entry.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: entry.name, path: entry.path };
    }

    contextMenuTargetFile = { name: entry.name, path: entry.path };
    contextMenuX = e.clientX;
    contextMenuY = e.clientY;
    showContextMenu = true;
  }

  // Playlist management
  function addToPlaylist(name: string, path: string) {
    if (playlistItems.some(item => item.path === path)) return;
    playlistItems = [...playlistItems, { name, path }];
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
  }

  function addSelectedToPlaylist() {
    selectedFilePaths.forEach(path => {
      const name = path.split("/").pop() || path;
      addToPlaylist(name, path);
    });
  }

  function removePlaylistItem(idx: number) {
    playlistItems = playlistItems.filter((_, i) => i !== idx);
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
  }

  function clearPlaylist() {
    playlistItems = [];
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
  }

  async function selectPlaylistFiles() {
    try {
      const selected = await open({
        multiple: true,
        filters: [{
          name: "Audio Files",
          extensions: ["wav", "mp3", "flac", "m4a", "aiff", "ogg"]
        }]
      });

      if (selected && Array.isArray(selected)) {
        selected.forEach(path => {
          const name = path.split("/").pop() || path;
          addToPlaylist(name, path);
        });
      } else if (selected && typeof selected === "string") {
        const name = selected.split("/").pop() || selected;
        addToPlaylist(name, selected);
      }
    } catch (err) {
      alert("Failed to add files: " + err);
    }
  }

  // File Associations Linkers
  async function associatePdfChart() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "PDF Charts", extensions: ["pdf"] }]
      });
      if (selected && typeof selected === "string") {
        pdfChartPath = selected;
        pdfChartName = selected.split("/").pop() || selected;
        localStorage.setItem("th_pdf_path", selected);
      }
    } catch (err) {
      alert("Failed to associate PDF: " + err);
    }
  }

  function clearPdfChart() {
    pdfChartPath = "";
    pdfChartName = "";
    localStorage.removeItem("th_pdf_path");
  }

  async function associateAlternativeVersion() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Audio Files", extensions: ["wav", "mp3", "flac", "m4a", "aiff", "ogg"] }]
      });
      if (selected && typeof selected === "string") {
        const name = selected.split("/").pop() || selected;
        associatedVersions = [...associatedVersions, { name, path: selected }];
        localStorage.setItem("th_associated_versions", JSON.stringify(associatedVersions));
      }
    } catch (err) {
      alert("Failed to associate track: " + err);
    }
  }

  function removeAssociatedVersion(idx: number) {
    associatedVersions = associatedVersions.filter((_, i) => i !== idx);
    localStorage.setItem("th_associated_versions", JSON.stringify(associatedVersions));
  }

  async function loadAssociatedTrack(path: string) {
    // Treat associated file load as alternate track
    await loadAudioPath(path, "alternate");
  }

  function getActiveTrack(): Track | null {
    if (activeTrackMode === "main") return mainTrack;
    if (activeTrackMode === "alternate") return alternateTrack;
    return null;
  }

  // Dynamic Peak Slice fetcher (Stereo Channels)
  async function updateVisiblePeaks() {
    const track = getActiveTrack();
    if (!track || !mainCanvas) {
      visibleChannels = [];
      return;
    }

    const rect = mainCanvas.getBoundingClientRect();
    const numPoints = Math.max(100, Math.floor(rect.width));

    const totalFrames = duration * sampleRate;
    const windowWidth = 1.0 / zoom;
    const startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2));
    const endProgress = startProgress + windowWidth;

    const startFrame = Math.floor(startProgress * totalFrames);
    const endFrame = Math.floor(endProgress * totalFrames);

    const visibleFrames = (endProgress - startProgress) * totalFrames;
    const isSampleLevel = visibleFrames < 1500;

    if (!isSampleLevel) {
      // Slicing locally from track.channelPeaks
      const newChannels: VisibleChannelData[] = [];
      for (let c = 0; c < track.channelPeaks.length; c++) {
        const cPeaks = track.channelPeaks[c];
        const peakLen = cPeaks.min.length;
        const startIdx = Math.floor(startProgress * peakLen);
        const endIdx = Math.floor(endProgress * peakLen);
        
        const minSlice = cPeaks.min.slice(startIdx, endIdx);
        const maxSlice = cPeaks.max.slice(startIdx, endIdx);

        if (minSlice.length <= numPoints) {
          newChannels.push({
            isSampleLevel: false,
            rawSamples: [],
            min: minSlice,
            max: maxSlice
          });
        } else {
          const downsampledMin: number[] = [];
          const downsampledMax: number[] = [];
          const chunkSize = minSlice.length / numPoints;
          for (let i = 0; i < numPoints; i++) {
            const chunkStart = Math.floor(i * chunkSize);
            const chunkEnd = Math.min(Math.floor((i + 1) * chunkSize), minSlice.length);
            if (chunkStart >= chunkEnd) break;
            let minVal = 0.0;
            let maxVal = 0.0;
            for (let j = chunkStart; j < chunkEnd; j++) {
              if (minSlice[j] < minVal) minVal = minSlice[j];
              if (maxSlice[j] > maxVal) maxVal = maxSlice[j];
            }
            downsampledMin.push(minVal);
            downsampledMax.push(maxVal);
          }
          newChannels.push({
            isSampleLevel: false,
            rawSamples: [],
            min: downsampledMin,
            max: downsampledMax
          });
        }
      }
      visibleChannels = newChannels;
      drawMainWaveform();
      drawOverviewWaveform();
      drawAltOverviewWaveform();
    } else {
      // Zoomed in extremely deep: fetch raw samples from backend
      try {
        const result: any = await invoke("get_waveform_slice", {
          startFrame: startFrame,
          endFrame: endFrame,
          numPoints: numPoints
        });
        const channelsData: number[][] = result.channels || [];
        visibleChannels = channelsData.map(samples => ({
          isSampleLevel: true,
          rawSamples: samples,
          min: [],
          max: []
        }));
        drawMainWaveform();
        drawOverviewWaveform();
        drawAltOverviewWaveform();
      } catch (err) {
        console.error("Failed to fetch raw samples", err);
      }
    }
  }

  // Playback handlers
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
    updateVisiblePeaks().then(() => {
      drawMainWaveform();
      drawOverviewWaveform();
      drawAltOverviewWaveform();
    });
  }

  async function handleVolume(e: Event) {
    const target = e.target as HTMLInputElement;
    dbVolume = parseFloat(target.value);
    volumeLinear = dbToLinear(dbVolume);
    await invoke("set_volume", { volume: volumeLinear });
  }

  function handleRewind() {
    invoke("seek", { seconds: 0 });
  }

  // Panning & Dragging State
  let isPanning = false;
  let panStartX = 0;
  let panStartProgress = 0;
  let hasDraggedMain = false;

  let isDraggingOverview = false;

  // Main Waveform: Mouse Drag to Pan, click to seek
  function handleMainMouseDown(e: MouseEvent) {
    if (duration === 0 || !mainCanvas) return;
    isPanning = true;
    hasDraggedMain = false;
    panStartX = e.clientX;
    panStartProgress = progress;
    
    window.addEventListener("mousemove", handleMainMouseMove);
    window.addEventListener("mouseup", handleMainMouseUp);
  }

  function handleMainMouseMove(e: MouseEvent) {
    if (!isPanning || !mainCanvas) return;
    const rect = mainCanvas.getBoundingClientRect();
    const width = rect.width;
    const windowWidth = 1.0 / zoom;
    const deltaX = e.clientX - panStartX;

    if (Math.abs(deltaX) > 3) {
      hasDraggedMain = true;
    }

    const progressChange = -(deltaX / width) * windowWidth;
    let newProgress = panStartProgress + progressChange;
    const halfWindow = windowWidth / 2;
    newProgress = Math.max(halfWindow, Math.min(1.0 - halfWindow, newProgress));

    progress = newProgress;
    currentTime = progress * duration;

    updateVisiblePeaks();
  }

  async function handleMainMouseUp(e: MouseEvent) {
    if (isPanning) {
      isPanning = false;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);

      if (!hasDraggedMain && mainCanvas) {
        // Simple Click: seek playhead to exactly where they clicked
        const rect = mainCanvas.getBoundingClientRect();
        const clickX = e.clientX - rect.left;
        const pct = clickX / rect.width;
        
        const windowWidth = 1.0 / zoom;
        const startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2));
        const targetPct = startProgress + pct * windowWidth;
        const targetSeconds = targetPct * duration;
        await invoke("seek", { seconds: targetSeconds });
      } else {
        // Drag finished: send the final seek position to synchronise audio engine playhead
        await invoke("seek", { seconds: progress * duration });
      }
    }
  }

  // Mouse wheel zoom on Main Waveform
  function handleMainWheel(e: WheelEvent) {
    if (duration === 0) return;
    e.preventDefault();
    // Zoom in/out logarithmically or linearly based on scroll delta
    const factor = e.deltaY < 0 ? 1.15 : 0.85;
    let newZoom = zoom * factor;
    newZoom = Math.max(1.0, Math.min(800.0, newZoom));
    
    // Soft snap to 15s notch if scrolling close to it
    if (Math.abs(newZoom - target15sZoom) < target15sZoom * 0.05) {
      newZoom = target15sZoom;
    }

    if (newZoom !== zoom) {
      zoom = newZoom;
      updateVisiblePeaks();
    }
  }

  function getNotchPercent(val: number, min = 1.0, max = 800.0): number {
    return Math.max(0, Math.min(100, ((val - min) / (max - min)) * 100));
  }

  function handleZoomInput(e: Event) {
    const inputVal = parseFloat((e.target as HTMLInputElement).value);
    const diff = Math.abs(inputVal - target15sZoom);
    if (diff < target15sZoom * 0.06) {
      zoom = target15sZoom;
    } else {
      zoom = inputVal;
    }
    updateVisiblePeaks();
  }

  // Overview Waveform: Drag highlighted window center
  function handleOverviewMouseDown(e: MouseEvent, target: "main" | "alternate") {
    if (target !== activeTrackMode) {
      toggleActiveTrack(target);
    }
    if (duration === 0) return;
    isDraggingOverview = true;
    
    updateOverviewDrag(e);
    window.addEventListener("mousemove", handleOverviewMouseMove);
    window.addEventListener("mouseup", handleOverviewMouseUp);
  }

  function handleOverviewMouseMove(e: MouseEvent) {
    if (!isDraggingOverview) return;
    updateOverviewDrag(e);
  }

  async function handleOverviewMouseUp(e: MouseEvent) {
    if (isDraggingOverview) {
      isDraggingOverview = false;
      window.removeEventListener("mousemove", handleOverviewMouseMove);
      window.removeEventListener("mouseup", handleOverviewMouseUp);
      // Synchronise engine playhead
      await invoke("seek", { seconds: progress * duration });
    }
  }

  function updateOverviewDrag(e: MouseEvent) {
    const canvas = activeTrackMode === "main" ? overviewCanvas : altOverviewCanvas;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const pct = Math.max(0, Math.min(1.0, clickX / rect.width));

    const windowWidth = 1.0 / zoom;
    const halfWindow = windowWidth / 2;

    progress = Math.max(halfWindow, Math.min(1.0 - halfWindow, pct));
    currentTime = progress * duration;

    updateVisiblePeaks();
  }

  // Markers
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
    const next = markers.find(m => m.time > currentTime + 0.1);
    if (next) {
      seekToMarker(next.time);
    }
  }

  function getActivePeaks(): number[] {
    if (activeTrackMode === "main" && mainTrack) {
      return mainTrack.peaks;
    } else if (activeTrackMode === "alternate" && alternateTrack) {
      return alternateTrack.peaks;
    }
    return [];
  }

  function setupCanvasResolution(canvas: HTMLCanvasElement, rect: DOMRect) {
    const dpr = window.devicePixelRatio || 1;
    const canvasWidth = Math.floor(rect.width * dpr);
    const canvasHeight = Math.floor(rect.height * dpr);

    if (canvas.width !== canvasWidth || canvas.height !== canvasHeight) {
      canvas.width = canvasWidth;
      canvas.height = canvasHeight;
    }
  }

  function formatRulerTime(seconds: number, stepSec: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    if (stepSec >= 1) {
      const s = Math.floor(secs);
      return `${mins}:${s < 10 ? '0' : ''}${s}`;
    } else if (stepSec >= 0.1) {
      const s = secs.toFixed(1);
      const prefix = secs < 10 ? '0' : '';
      return `${mins}:${prefix}${s}`;
    } else {
      const s = secs.toFixed(2);
      const prefix = secs < 10 ? '0' : '';
      return `${mins}:${prefix}${s}`;
    }
  }

  // Draw Main Waveform (Split Stereo, Dynamic Time Ruler, Oscillating Curve)
  function drawMainWaveform() {
    if (!mainCanvas) return;
    const ctx = mainCanvas.getContext("2d");
    if (!ctx) return;

    const rect = mainCanvas.getBoundingClientRect();
    setupCanvasResolution(mainCanvas, rect);

    const dpr = window.devicePixelRatio || 1;
    const width = rect.width;
    const height = rect.height;

    ctx.clearRect(0, 0, mainCanvas.width, mainCanvas.height);
    ctx.save();
    ctx.scale(dpr, dpr);

    // 1. Background Fill
    ctx.fillStyle = "#0c151e";
    ctx.fillRect(0, 0, width, height);

    const track = getActiveTrack();
    if (!track || visibleChannels.length === 0 || duration === 0) {
      ctx.fillStyle = "#888888";
      ctx.font = "13px sans-serif";
      ctx.fillText("No active track loaded (Drag & Drop file to load)", width / 2 - 140, height / 2 + 4);
      ctx.restore();
      return;
    }

    const windowWidth = 1.0 / zoom;
    const startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2));
    const endProgress = startProgress + windowWidth;

    const startTime = startProgress * duration;
    const endTime = endProgress * duration;
    const visibleSec = endTime - startTime;

    // 2. Ruler & Grid Configuration
    const rulerHeight = 20;
    let majorStep = 10;
    let minorStep = 2;

    if (visibleSec > 180) {
      majorStep = 60;
      minorStep = 30;
    } else if (visibleSec > 60) {
      majorStep = 30;
      minorStep = 10;
    } else if (visibleSec > 20) {
      majorStep = 10;
      minorStep = 2;
    } else if (visibleSec > 8) {
      majorStep = 5;
      minorStep = 1;
    } else if (visibleSec > 2) {
      majorStep = 1;
      minorStep = 0.2;
    } else if (visibleSec > 0.5) {
      majorStep = 0.5;
      minorStep = 0.1;
    } else {
      majorStep = 0.1;
      minorStep = 0.02;
    }

    // Draw Background Grid Lines
    const firstMinor = Math.floor(startTime / minorStep) * minorStep;
    for (let t = firstMinor; t <= endTime + minorStep; t += minorStep) {
      if (t < startTime || t > endTime) continue;
      const x = ((t - startTime) / visibleSec) * width;
      const isMajor = Math.abs(Math.round(t / majorStep) * majorStep - t) < 0.0001;

      ctx.strokeStyle = isMajor ? "rgba(255, 255, 255, 0.12)" : "rgba(255, 255, 255, 0.04)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, rulerHeight);
      ctx.lineTo(x, height);
      ctx.stroke();
    }

    // 3. Top Ruler Header Bar
    ctx.fillStyle = "#091017";
    ctx.fillRect(0, 0, width, rulerHeight);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.1)";
    ctx.beginPath();
    ctx.moveTo(0, rulerHeight);
    ctx.lineTo(width, rulerHeight);
    ctx.stroke();

    // Ruler Ticks and Labels
    ctx.fillStyle = "#7b9bb6";
    ctx.font = "9px -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif";
    const firstMajor = Math.floor(startTime / majorStep) * majorStep;
    for (let t = firstMajor; t <= endTime + majorStep; t += majorStep) {
      if (t < startTime || t > endTime) continue;
      const x = ((t - startTime) / visibleSec) * width;
      
      // Tick mark
      ctx.strokeStyle = "#5a7a94";
      ctx.beginPath();
      ctx.moveTo(x, rulerHeight - 5);
      ctx.lineTo(x, rulerHeight);
      ctx.stroke();

      // Text label
      const label = formatRulerTime(t, majorStep);
      ctx.fillText(label, x + 4, rulerHeight - 7);
    }

    // 4. Split Stereo Channels Waveform
    const waveTop = rulerHeight;
    const waveHeight = height - rulerHeight;
    const numChannels = visibleChannels.length;
    const channelHeight = waveHeight / numChannels;

    for (let c = 0; c < numChannels; c++) {
      const chData = visibleChannels[c];
      const chTop = waveTop + c * channelHeight;
      const chCenter = chTop + channelHeight / 2;

      // Channel boundary line
      if (c > 0) {
        ctx.strokeStyle = "rgba(255, 255, 255, 0.1)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(0, chTop);
        ctx.lineTo(width, chTop);
        ctx.stroke();
      }

      // Zero-Crossing Baseline
      ctx.strokeStyle = "rgba(59, 153, 252, 0.25)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(0, chCenter);
      ctx.lineTo(width, chCenter);
      ctx.stroke();

      // Channel Badge (L / R)
      if (numChannels > 1) {
        ctx.fillStyle = "#4a6c87";
        ctx.font = "bold 9px sans-serif";
        ctx.fillText(c === 0 ? "L" : "R", 8, chTop + 14);
      }

      // Signal Plotting
      if (chData.isSampleLevel && chData.rawSamples.length > 0) {
        // Sample Level: Smooth continuous oscillating signal line
        const samples = chData.rawSamples;
        const step = width / (samples.length - 1 || 1);

        ctx.strokeStyle = "#3b99fc";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < samples.length; i++) {
          const x = i * step;
          const y = chCenter - samples[i] * (channelHeight * 0.44);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();

        // Sample Node Dots
        ctx.fillStyle = "#8fc4fa";
        for (let i = 0; i < samples.length; i++) {
          const x = i * step;
          const y = chCenter - samples[i] * (channelHeight * 0.44);
          ctx.fillRect(x - 2, y - 2, 4, 4);
        }
      } else if (chData.min.length > 0) {
        // True Bipolar Min/Max Vertical Envelope Bars
        const step = width / chData.min.length;
        const gap = step > 4 ? 1.5 : (step > 2 ? 1 : 0);
        const barWidth = Math.max(1, step - gap);

        ctx.fillStyle = "#3b99fc";

        for (let i = 0; i < chData.min.length; i++) {
          const minVal = chData.min[i]; // Negative or 0
          const maxVal = chData.max[i]; // Positive or 0
          const x = i * step;

          const yTop = chCenter - maxVal * (channelHeight * 0.44);
          const yBottom = chCenter - minVal * (channelHeight * 0.44);
          const barH = Math.max(1, yBottom - yTop);

          ctx.fillRect(x, yTop, barWidth, barH);
        }
      }
    }

    // 5. Markers
    for (const marker of markers) {
      const markerPct = marker.time / duration;
      if (markerPct >= startProgress && markerPct <= endProgress) {
        const markerX = ((markerPct - startProgress) / windowWidth) * width;
        ctx.strokeStyle = "#ff9500"; 
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(markerX, rulerHeight);
        ctx.lineTo(markerX, height);
        ctx.stroke();
        
        ctx.fillStyle = "#ff9500";
        ctx.font = "bold 9px sans-serif";
        ctx.fillText(marker.name, markerX + 4, rulerHeight + 12);
      }
    }

    // 6. Playhead Indicator (with Ruler Pointer Triangle)
    const playheadPct = (progress - startProgress) / windowWidth;
    const playheadX = playheadPct * width;

    if (playheadPct >= 0 && playheadPct <= 1.0) {
      // Playhead Line
      ctx.strokeStyle = "#ffcc00"; 
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(playheadX, 0);
      ctx.lineTo(playheadX, height);
      ctx.stroke();

      // Top Ruler Triangle Head
      ctx.fillStyle = "#ffcc00";
      ctx.beginPath();
      ctx.moveTo(playheadX - 5, 0);
      ctx.lineTo(playheadX + 5, 0);
      ctx.lineTo(playheadX, 7);
      ctx.closePath();
      ctx.fill();
    }

    ctx.restore();
  }

  function drawGenericOverview(canvas: HTMLCanvasElement, track: Track | null, mode: "main" | "alternate") {
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const rect = canvas.getBoundingClientRect();
    setupCanvasResolution(canvas, rect);

    const dpr = window.devicePixelRatio || 1;
    const width = rect.width;
    const height = rect.height;

    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.save();
    ctx.scale(dpr, dpr);

    const isActive = activeTrackMode === mode;
    ctx.fillStyle = isActive ? "#262626" : "#1a1a1a";
    ctx.fillRect(0, 0, width, height);

    if (!track) {
      ctx.fillStyle = "#666666";
      ctx.font = "10px sans-serif";
      ctx.fillText(`Empty [Double click file in browser to load as ${mode.toUpperCase()}]`, 12, height / 2 + 3);
      ctx.restore();
      return;
    }

    const peaks = track.overviewPeaks || [];
    const barWidth = width / peaks.length;
    const halfHeight = height / 2;
    ctx.fillStyle = isActive ? "#a8c3d8" : "#556673";
    for (let i = 0; i < peaks.length; i += 2) {
      const val = peaks[i];
      const barHeight = val * (height * 0.7);
      const x = i * barWidth;
      const y = halfHeight - barHeight / 2;
      ctx.fillRect(x, y, Math.max(1, barWidth * 2 - 0.5), barHeight);
    }

    if (isActive) {
      const windowWidth = 1.0 / zoom;
      const startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2));
      const endProgress = startProgress + windowWidth;

      ctx.fillStyle = "rgba(59, 153, 252, 0.18)"; 
      ctx.fillRect(startProgress * width, 0, (endProgress - startProgress) * width, height);
      
      ctx.strokeStyle = "#3b99fc";
      ctx.lineWidth = 1;
      ctx.strokeRect(startProgress * width, 0, (endProgress - startProgress) * width, height);

      const playheadX = progress * width;
      ctx.strokeStyle = "#3b99fc";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(playheadX, 0);
      ctx.lineTo(playheadX, height);
      ctx.stroke();
    }

    ctx.restore();
  }

  function drawOverviewWaveform() {
    drawGenericOverview(overviewCanvas, mainTrack, "main");
  }

  function drawAltOverviewWaveform() {
    drawGenericOverview(altOverviewCanvas, alternateTrack, "alternate");
  }

  // Custom Knobs Mouse Interaction
  function handleKnobMousedown(
    e: MouseEvent,
    id: string,
    val: number,
    min: number,
    max: number,
    step: number,
    setValue: (v: number) => void
  ) {
    e.preventDefault();
    activeKnob = {
      id,
      startY: e.clientY,
      startX: e.clientX,
      startVal: val,
      min,
      max,
      step,
      setValue
    };
    window.addEventListener("mousemove", handleKnobMousemove);
    window.addEventListener("mouseup", handleKnobMouseup);
  }

  function handleKnobMousemove(e: MouseEvent) {
    if (!activeKnob) return;
    const dy = activeKnob.startY - e.clientY;
    const dx = e.clientX - activeKnob.startX;
    
    // Sensitivity scalar
    const pixelsPerRange = 150;
    const range = activeKnob.max - activeKnob.min;
    const delta = ((dy + dx) / pixelsPerRange) * range;
    
    let newVal = activeKnob.startVal + delta;
    newVal = Math.max(activeKnob.min, Math.min(activeKnob.max, newVal));
    newVal = Math.round(newVal / activeKnob.step) * activeKnob.step;
    activeKnob.setValue(newVal);
  }

  function handleKnobMouseup() {
    activeKnob = null;
    window.removeEventListener("mousemove", handleKnobMousemove);
    window.removeEventListener("mouseup", handleKnobMouseup);
  }

  function getKnobRotation(val: number, min: number, max: number) {
    const pct = (val - min) / (max - min);
    return -135 + pct * 270;
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

<svelte:window on:keydown={handleBrowserKeydown} />

<main class="app-container">
  <!-- Top bar -->
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
    
    <!-- LEFT SIDEBAR: Switchable Browser & Playlist -->
    <aside class="sidebar-left">
      
      <!-- Tab Selector -->
      <div class="tab-selectors">
        <button 
          class="tab-btn" 
          class:active={activeTab === "browser"} 
          on:click={() => activeTab = "browser"}
        >
          BROWSER
        </button>
        <button 
          class="tab-btn" 
          class:active={activeTab === "playlist"} 
          on:click={() => activeTab = "playlist"}
        >
          PLAYLIST
        </button>
      </div>

      <!-- Tab Content: Browser -->
      {#if activeTab === "browser"}
        <div class="browser-nav">
          <span class="current-dir-label" title={currentPath}>{currentPath.split("/").pop() || currentPath}</span>
          {#if parentPath}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <span class="up-btn" on:click={() => loadBrowser(parentPath)}>Parent ↰</span>
          {/if}
        </div>

        <input 
          type="text" 
          placeholder="Search / filter files..." 
          bind:value={searchQuery} 
          class="browser-search-input" 
        />

        <div class="browser-list">
          {#each filteredEntries as entry}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div 
              class="browser-item" 
              class:is-dir={entry.is_dir}
              class:active={selectedFilePaths.has(entry.path)}
              on:click={(e) => handleFileClick(e, entry)}
              on:contextmenu={(e) => handleContextMenu(e, entry)}
              on:dblclick={() => {
                if (entry.is_dir) {
                  loadBrowser(entry.path);
                } else {
                  loadAudioPath(entry.path, "main");
                }
              }}
            >
              <span class="item-icon">{entry.is_dir ? "📁" : "🎵"}</span>
              <span class="item-name" title={entry.name}>{entry.name}</span>
            </div>
          {/each}
        </div>
      {:else}
        <!-- Tab Content: Playlist -->
        <div class="playlist-list">
          {#if playlistItems.length === 0}
            <div class="placeholder-text-sidebar">Playlist is empty. Add files below or right-click files in Browser.</div>
          {:else}
            {#each playlistItems as item, idx}
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div 
                class="playlist-item-sidebar"
                class:active={filePath === item.path}
                on:dblclick={() => loadAudioPath(item.path, "main")}
              >
                <span class="item-icon">🎵</span>
                <span class="item-name" title={item.name}>{item.name}</span>
                <button class="remove-playlist-item-btn" on:click={() => removePlaylistItem(idx)}>×</button>
              </div>
            {/each}
          {/if}
        </div>

        <div class="playlist-controls-sidebar">
          <button class="action-btn file-btn" on:click={selectPlaylistFiles}>
            + Add Files
          </button>
          <button class="action-btn clear-btn" on:click={clearPlaylist}>
            Clear List
          </button>
        </div>
      {/if}

      <div class="selection-info">
        <div class="panel-header">ACTIVE SELECTION INFO</div>
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
    </aside>

    <!-- CENTER AREA: Resizable Waveforms & Fixed Bottom controls -->
    <section class="center-content" bind:this={centerContentElement}>
      
      <!-- Current File info header -->
      <div class="track-header">
        <div class="track-title-info">
          <h2>
            <span class="track-badge" class:main-badge={activeTrackMode === "main"}>
              {activeTrackMode.toUpperCase()}
            </span>
            {fileName || "Drag & Drop Audio file to begin"}
          </h2>
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

      <!-- Waveforms (flex-grow vertically) -->
      <div class="waveforms-flexbox">
        <!-- Overview Waveform (Alternate File) -->
        <div class="waveform-block block-alt-overview">
          <div class="waveform-label-row">
            <span class="waveform-label">WAVEFORM OVERVIEW (ALTERNATE FILE)</span>
            {#if alternateTrack}
              <span class="overview-desc-tag" class:active={activeTrackMode === "alternate"}>
                Double-click to activate
              </span>
            {/if}
          </div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={altOverviewCanvas} 
            on:mousedown={(e) => handleOverviewMouseDown(e, "alternate")}
            on:dblclick={() => toggleActiveTrack("alternate")}
            class="overview-canvas"
          ></canvas>
        </div>

        <!-- Overview Waveform (Main File) -->
        <div class="waveform-block block-main-overview">
          <div class="waveform-label-row">
            <span class="waveform-label">WAVEFORM OVERVIEW (MAIN FILE)</span>
            {#if mainTrack}
              <span class="overview-desc-tag" class:active={activeTrackMode === "main"}>
                Double-click to activate
              </span>
            {/if}
          </div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={overviewCanvas} 
            on:mousedown={(e) => handleOverviewMouseDown(e, "main")}
            on:dblclick={() => toggleActiveTrack("main")}
            class="overview-canvas"
          ></canvas>
        </div>

        <!-- Main Waveform Box (Flex-grow to occupy rest of center height) -->
        <div class="waveform-block block-main-waveform">
          <div class="waveform-label">MAIN WAVEFORM DISPLAY (ZOOMED & SCROLLING)</div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={mainCanvas} 
            on:mousedown={handleMainMouseDown}
            on:wheel|passive={handleMainWheel}
            class="main-canvas"
          ></canvas>
        </div>
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

        <!-- Looping, markers, and Zoom controls -->
        <div class="control-group">
          <div class="group-label">MARKERS / LOOPING / ZOOM (Dbl-click slider resets)</div>
          <div class="loop-zoom-grid">
            <div class="btn-row">
              <button class="control-btn accent-btn" on:click={addMarker}>+ Add Marker</button>
              <button class="control-btn disabled-btn" title="Loop start (Milestone 5)">[ Loop</button>
              <button class="control-btn disabled-btn" title="Loop end (Milestone 5)">Loop ]</button>
            </div>
            <div class="zoom-slider-group">
              <span class="control-text-label">ZOOM</span>
              <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
              <div class="zoom-track-wrapper">
                <input 
                  type="range" 
                  min="1.0" 
                  max="800.0" 
                  step="0.5" 
                  bind:value={zoom} 
                  on:dblclick={() => { zoom = target15sZoom; updateVisiblePeaks(); }}
                  on:input={handleZoomInput}
                  class="zoom-slider" 
                />
                {#if duration > 15}
                  <div 
                    class="zoom-snap-notch" 
                    style="left: {getNotchPercent(target15sZoom)}%;" 
                    title="15s Rehearsal View"
                  ></div>
                {/if}
              </div>
              <span class="zoom-value-label">{zoom.toFixed(0)}x</span>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- RIGHT SIDEBAR: Markers, FX, Project Setup (Dorico Theme) -->
    <aside class="sidebar-right">
      
      <!-- Project and File Associations -->
      <div class="panel-section">
        <div class="panel-header">PROJECT SETUP</div>
        <div class="linked-files">
          <div class="setup-row">
            <span class="label">Main Track:</span>
            <span class="value" class:active-val={mainTrack}>{mainTrack ? "LOADED" : "EMPTY"}</span>
          </div>
          <div class="setup-row mb-12">
            <span class="label">Alt Track:</span>
            <span class="value" class:active-val={alternateTrack}>{alternateTrack ? "LOADED" : "EMPTY"}</span>
          </div>

          <div class="associations-divider">FILE ASSOCIATIONS</div>
          
          <!-- PDF Chart Association -->
          <div class="assoc-block">
            <div class="assoc-title-row">
              <span class="assoc-label">PDF Chart:</span>
              {#if pdfChartPath}
                <button class="clear-assoc-btn" on:click={clearPdfChart}>Clear</button>
              {/if}
            </div>
            {#if pdfChartPath}
              <span class="assoc-value" title={pdfChartPath}>📄 {pdfChartName}</span>
            {:else}
              <button class="assoc-add-btn" on:click={associatePdfChart}>Link PDF Chart...</button>
            {/if}
          </div>

          <!-- Other Associated Versions -->
          <div class="assoc-block mt-8">
            <div class="assoc-title-row">
              <span class="assoc-label">Associated Tracks:</span>
              <button class="clear-assoc-btn" on:click={associateAlternativeVersion}>+ Link</button>
            </div>
            <div class="associated-versions-list">
              {#if associatedVersions.length === 0}
                <span class="placeholder-text">None linked (e.g. vocals-only, live).</span>
              {:else}
                {#each associatedVersions as version, idx}
                  <div class="assoc-version-item">
                    <!-- svelte-ignore a11y-click-events-have-key-events -->
                    <!-- svelte-ignore a11y-no-static-element-interactions -->
                    <span 
                      class="assoc-version-name" 
                      title="Load as alternate track"
                      on:click={() => loadAssociatedTrack(version.path)}
                    >
                      {version.name}
                    </span>
                    <button class="remove-assoc-btn" on:click={() => removeAssociatedVersion(idx)}>×</button>
                  </div>
                {/each}
              {/if}
            </div>
          </div>
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

      <!-- DSP Effects Rack Panel (Knobs layout from BOTTOM up) -->
      <div class="panel-section dsp-section">
        <div class="panel-header">EFFECTS MODULES</div>

        <!-- 4. Compressor knobs (Threshold, Knee, Makeup) -->
        <div class="knobs-row placeholder-knobs">
          <!-- Threshold -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "comp_thresh", compressorThreshold, -60, 0, 1, (v) => compressorThreshold = v)}
            on:dblclick={() => compressorThreshold = -20}
            title="Double-click resets to -20 dB"
          >
            <span class="knob-label">Threshold</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorThreshold, -60, 0)}deg)"></div>
            </div>
            <span class="knob-value">{compressorThreshold} dB</span>
          </div>

          <!-- Ratio -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "comp_ratio", compressorRatio, 1.0, 20.0, 0.5, (v) => compressorRatio = v)}
            on:dblclick={() => compressorRatio = 2.0}
            title="Double-click resets to 2.0:1"
          >
            <span class="knob-label">Ratio</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorRatio, 1.0, 20.0)}deg)"></div>
            </div>
            <span class="knob-value">{compressorRatio.toFixed(1)}:1</span>
          </div>

          <!-- Makeup -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "comp_makeup", compressorMakeup, 0, 24, 0.5, (v) => compressorMakeup = v)}
            on:dblclick={() => compressorMakeup = 0}
            title="Double-click resets to 0 dB"
          >
            <span class="knob-label">Makeup</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorMakeup, 0, 24)}deg)"></div>
            </div>
            <span class="knob-value">+{compressorMakeup.toFixed(1)} dB</span>
          </div>
        </div>

        <!-- 3. Equalizer knobs (Bass, Treble) -->
        <div class="knobs-row placeholder-knobs">
          <!-- Bass -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "eq_bass", eqBass, -12, 12, 0.5, (v) => eqBass = v)}
            on:dblclick={() => eqBass = 0}
            title="Double-click resets to 0 dB"
          >
            <span class="knob-label">Bass</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(eqBass, -12, 12)}deg)"></div>
            </div>
            <span class="knob-value">{eqBass > 0 ? "+" : ""}{eqBass.toFixed(1)} dB</span>
          </div>

          <!-- Treble -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "eq_treble", eqTreble, -12, 12, 0.5, (v) => eqTreble = v)}
            on:dblclick={() => eqTreble = 0}
            title="Double-click resets to 0 dB"
          >
            <span class="knob-label">Treble</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(eqTreble, -12, 12)}deg)"></div>
            </div>
            <span class="knob-value">{eqTreble > 0 ? "+" : ""}{eqTreble.toFixed(1)} dB</span>
          </div>
        </div>

        <!-- 2. Speed and Pitch Knobs (Knobs on same line) -->
        <div class="knobs-row active-knobs">
          <!-- Speed Knob -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "speed", speed, 0.25, 4.00, 0.05, (v) => speed = v)}
            on:dblclick={() => speed = 1.0}
            title="Double-click resets to 100%"
          >
            <span class="knob-label">Speed</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(speed, 0.25, 4.00)}deg)"></div>
            </div>
            <span class="knob-value">{Math.round(speed * 100)}%</span>
          </div>

          <!-- Pitch Knob -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "pitch", pitch, -24, 24, 1, (v) => pitch = v)}
            on:dblclick={() => pitch = 0}
            title="Double-click resets to 0 semitones"
          >
            <span class="knob-label">Pitch Shift</span>
            <div class="knob-circle">
              <div class="knob-marker" style="transform: rotate({getKnobRotation(pitch, -24, 24)}deg)"></div>
            </div>
            <span class="knob-value">{pitch > 0 ? "+" : ""}{pitch} st</span>
          </div>
        </div>

        <!-- 1. Volume Master Gain (Slider at the very bottom) -->
        <div class="dsp-control active-dsp">
          <div class="dsp-label-row">
            <span class="dsp-title">VOLUME GAIN</span>
            <span class="dsp-value">{dbVolume <= -59.5 ? "-inf" : (dbVolume > 0 ? "+" : "") + dbVolume.toFixed(1)} dB</span>
          </div>
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <input 
            type="range" 
            min="-60.0" 
            max="12.0" 
            step="0.5" 
            value={dbVolume} 
            on:input={handleVolume}
            on:dblclick={async () => {
              dbVolume = 0.0;
              volumeLinear = 1.0;
              await invoke("set_volume", { volume: 1.0 });
            }}
            class="dsp-slider" 
          />
        </div>

      </div>
    </aside>

  </div>

  <!-- Custom Right-click Context Menu -->
  {#if showContextMenu}
    <div 
      class="context-menu" 
      style="top: {contextMenuY}px; left: {contextMenuX}px;"
      on:click|stopPropagation
    >
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <!-- svelte-ignore a11y-no-static-element-interactions -->
      <div class="menu-item" on:click={() => { if (contextMenuTargetFile) { loadAudioPath(contextMenuTargetFile.path, "main"); showContextMenu = false; } }}>
        Set as Main Track
      </div>
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <!-- svelte-ignore a11y-no-static-element-interactions -->
      <div class="menu-item" on:click={() => { if (contextMenuTargetFile) { loadAudioPath(contextMenuTargetFile.path, "alternate"); showContextMenu = false; } }}>
        Set as Alt Track
      </div>
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <!-- svelte-ignore a11y-no-static-element-interactions -->
      <div class="menu-item" on:click={() => { if (contextMenuTargetFile) { addToPlaylist(contextMenuTargetFile.name, contextMenuTargetFile.path); showContextMenu = false; } }}>
        Add to Current Playlist
      </div>
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <!-- svelte-ignore a11y-no-static-element-interactions -->
      <div class="menu-item" on:click={() => { addSelectedToPlaylist(); showContextMenu = false; }}>
        Add Selected to Playlist ({selectedFilePaths.size})
      </div>
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <!-- svelte-ignore a11y-no-static-element-interactions -->
      <div class="menu-item" on:click={() => { if (contextMenuTargetFile) { clearPlaylist(); addToPlaylist(contextMenuTargetFile.name, contextMenuTargetFile.path); showContextMenu = false; } }}>
        Create New Playlist from File
      </div>
    </div>
  {/if}
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
    max-width: 400px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* Left Sidebar Tab system */
  .sidebar-left {
    background-color: #252526;
    border-right: 1px solid #3c3c3c;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .tab-selectors {
    display: flex;
    background-color: #1f1f20;
    border-bottom: 1px solid #3c3c3c;
  }

  .tab-btn {
    flex-grow: 1;
    background: transparent;
    border: none;
    color: #8e8e8e;
    padding: 10px;
    font-size: 0.75rem;
    font-weight: bold;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: all 0.15s ease;
  }

  .tab-btn.active {
    color: #3b99fc;
    border-bottom-color: #3b99fc;
    background-color: #252526;
  }

  .browser-nav {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #202021;
    border-bottom: 1px solid #3c3c3c;
    padding: 3px 8px;
    font-size: 0.72rem;
  }

  .current-dir-label {
    font-weight: 700;
    color: #ffffff;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .up-btn {
    color: #3b99fc;
    cursor: pointer;
    font-weight: bold;
  }

  .up-btn:hover {
    text-decoration: underline;
  }

  .browser-search-input {
    background-color: #1a1a1b;
    border: 1px solid #3c3c3c;
    border-radius: 4px;
    color: #ffffff;
    padding: 3px 8px;
    font-size: 0.72rem;
    margin: 4px 8px;
    width: calc(100% - 16px);
    box-sizing: border-box;
    outline: none;
    transition: border-color 0.15s ease;
  }

  .browser-search-input:focus {
    border-color: #3b99fc;
  }

  .browser-list {
    flex-grow: 1;
    overflow-y: auto;
    background-color: #1e1e1f;
  }

  .browser-item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0px 8px;
    font-size: 0.72rem;
    cursor: pointer;
    color: #ffffff; /* White text */
    transition: background-color 0.15s ease;
  }

  .browser-item:nth-child(even) {
    background-color: #1a1a1b;
  }

  .browser-item:nth-child(odd) {
    background-color: #212122;
  }

  .browser-item:hover {
    background-color: #2d2d2e;
  }

  .browser-item.is-dir {
    font-weight: 600;
    color: #ffffff; /* White text for directories */
  }

  .browser-item.active {
    background-color: #333d46;
    color: #ffffff;
  }

  .browser-item.active.is-dir {
    color: #ffffff;
  }

  /* Playlist Sidebar mode */
  .playlist-list {
    flex-grow: 1;
    overflow-y: auto;
    background-color: #1e1e1f;
    padding: 6px 0;
  }

  .placeholder-text-sidebar {
    font-size: 0.75rem;
    color: #717171;
    padding: 16px;
    font-style: italic;
    line-height: 1.4;
  }

  .playlist-item-sidebar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0px 10px;
    font-size: 0.72rem;
    cursor: pointer;
    color: #cccccc;
    transition: background-color 0.15s ease;
  }

  .playlist-item-sidebar:hover {
    background-color: #2d2d2e;
  }

  .playlist-item-sidebar.active {
    background-color: #333d46;
    color: #ffffff;
    border-left: 3px solid #3b99fc;
  }

  .remove-playlist-item-btn {
    background: transparent;
    border: none;
    color: #717171;
    cursor: pointer;
    margin-left: auto;
    font-size: 1rem;
    font-weight: bold;
  }

  .remove-playlist-item-btn:hover {
    color: #ff453a;
  }

  .playlist-controls-sidebar {
    background-color: #202021;
    border-top: 1px solid #3c3c3c;
    padding: 12px;
    display: flex;
    gap: 8px;
  }

  .clear-btn {
    background-color: #333333;
    color: #d1d1d1;
    border: 1px solid #444444;
  }

  .clear-btn:hover {
    background-color: #444444;
    color: #ffffff;
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



  /* Center Workspace area styling */
  .center-content {
    background-color: #1e1e1e;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow: hidden;
    height: 100%;
    box-sizing: border-box;
  }

  .track-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid #2d2d2d;
    padding-bottom: 8px;
    flex-shrink: 0;
  }

  h2 {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 600;
    color: #ffffff;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .track-badge {
    font-size: 0.65rem;
    font-weight: 850;
    letter-spacing: 0.05em;
    padding: 2px 6px;
    border-radius: 3px;
    background-color: #ff9500;
    color: #000000;
  }

  .track-badge.main-badge {
    background-color: #3b99fc;
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

  /* Waveforms stretching wrapper */
  .waveforms-flexbox {
    display: flex;
    flex-direction: column;
    flex-grow: 1;
    gap: 10px;
    overflow: hidden;
  }

  .waveform-block {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .block-alt-overview, .block-main-overview {
    flex-shrink: 0;
    height: 65px;
  }

  .block-main-waveform {
    flex-grow: 1;
    background-color: #122a3a; 
    border: 1px solid #1b384d;
    border-radius: 4px;
  }

  .waveform-label-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 2px;
  }

  .waveform-label {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #8e8e8e;
  }

  .overview-desc-tag {
    font-size: 0.6rem;
    color: #717171;
    font-style: italic;
  }

  .overview-desc-tag.active {
    color: #3b99fc;
    font-weight: bold;
    font-style: normal;
  }

  .overview-canvas {
    display: block;
    width: 100%;
    height: 45px;
    cursor: pointer;
    border: 1px solid #3c3c3c;
    border-radius: 4px;
  }

  .main-canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: pointer;
  }

  /* Controls row */
  .controls-row {
    display: flex;
    gap: 16px;
    flex-shrink: 0;
    margin-top: 5px;
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
    background-color: #ff9500; 
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

  /* Loop & Zoom layout */
  .loop-zoom-grid {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .zoom-slider-group {
    display: flex;
    align-items: center;
    gap: 10px;
    background-color: #1e1e1e;
    border: 1px solid #3c3c3c;
    padding: 4px 12px;
    border-radius: 20px;
  }

  .zoom-track-wrapper {
    position: relative;
    display: flex;
    align-items: center;
  }

  .zoom-snap-notch {
    position: absolute;
    top: -3px;
    width: 2px;
    height: 10px;
    background-color: #ff9500;
    opacity: 0.8;
    pointer-events: none;
    transform: translateX(-50%);
    border-radius: 1px;
  }

  .control-text-label {
    font-size: 0.65rem;
    font-weight: 800;
    color: #8e8e8e;
  }

  .zoom-slider {
    -webkit-appearance: none;
    appearance: none;
    background: #3c3c3c;
    height: 4px;
    border-radius: 2px;
    outline: none;
    width: 120px;
  }

  .zoom-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #ff9500;
    cursor: pointer;
    position: relative;
    z-index: 2;
  }

  .zoom-value-label {
    font-size: 0.75rem;
    font-family: monospace;
    color: #ffffff;
    width: 38px;
    text-align: right;
  }

  /* Right Sidebar styles */
  .sidebar-right {
    background-color: #252526; 
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
    padding: 6px 12px;
    display: flex;
    flex-direction: column;
  }

  .setup-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.75rem;
    margin-bottom: 4px;
  }

  .setup-row .label {
    color: #8e8e8e;
  }

  .setup-row .value {
    font-weight: bold;
    color: #717171;
  }

  .setup-row .value.active-val {
    color: #3b99fc;
  }

  .mb-12 {
    margin-bottom: 12px;
  }

  /* Project File Associations section */
  .associations-divider {
    font-size: 0.65rem;
    font-weight: 800;
    color: #8e8e8e;
    letter-spacing: 0.08em;
    border-top: 1px solid #333333;
    padding-top: 12px;
    margin-bottom: 8px;
  }

  .assoc-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .assoc-title-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .assoc-label {
    font-size: 0.75rem;
    font-weight: 600;
    color: #aaaaaa;
  }

  .clear-assoc-btn {
    background: transparent;
    border: none;
    color: #ff453a;
    cursor: pointer;
    font-size: 0.7rem;
    padding: 0;
  }

  .clear-assoc-btn:hover {
    text-decoration: underline;
  }

  .assoc-value {
    font-size: 0.75rem;
    color: #3b99fc;
    word-break: break-all;
    background-color: #1e1e1e;
    padding: 4px 8px;
    border-radius: 4px;
    border: 1px solid #3c3c3c;
  }

  .assoc-add-btn {
    background-color: #2c2c2d;
    color: #d1d1d1;
    border: 1px dashed #444444;
    padding: 6px;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
    text-align: left;
    width: 100%;
  }

  .assoc-add-btn:hover {
    background-color: #3c3c3e;
    color: #ffffff;
  }

  .mt-8 { margin-top: 8px; }

  .associated-versions-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
  }

  .assoc-version-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #1e1e1e;
    border: 1px solid #3c3c3c;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 0.75rem;
  }

  .assoc-version-name {
    color: #3b99fc;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 190px;
  }

  .assoc-version-name:hover {
    text-decoration: underline;
  }

  .remove-assoc-btn {
    background: transparent;
    border: none;
    color: #717171;
    cursor: pointer;
    font-size: 0.9rem;
    font-weight: bold;
  }

  .remove-assoc-btn:hover {
    color: #ff453a;
  }

  .placeholder-text {
    font-size: 0.7rem;
    color: #717171;
    font-style: italic;
    line-height: 1.4;
  }

  .markers-section {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .markers-list {
    flex-grow: 1;
    overflow-y: auto;
    padding: 4px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .marker-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background-color: #2d2d2d;
    border: 1px solid #3c3c3c;
    padding: 3px 6px;
    border-radius: 4px;
    font-size: 0.75rem;
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

  /* DSP Panel Knobs styling */
  .dsp-section {
    padding-bottom: 6px;
  }

  .dsp-control {
    padding: 4px 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .dsp-label-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.7rem;
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
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #3b99fc;
    cursor: pointer;
  }

  .dsp-slider::-webkit-slider-thumb:hover {
    background: #5faeff;
  }

  /* Knobs row */
  .knobs-row {
    display: flex;
    justify-content: space-around;
    padding: 6px 4px;
    background-color: #1e1e1f;
    border-bottom: 1px solid #2d2d2d;
  }

  .knob-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    cursor: ns-resize;
    width: 55px;
  }

  .knob-label {
    font-size: 0.6rem;
    color: #8e8e8e;
    margin-bottom: 2px;
    text-align: center;
    white-space: nowrap;
  }

  .knob-circle {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background-color: #333333;
    border: 1.5px solid #555555;
    position: relative;
    box-shadow: inset 0 2px 4px rgba(0,0,0,0.5);
    margin-bottom: 2px;
    transition: border-color 0.15s ease;
  }

  .knob-container:hover .knob-circle {
    border-color: #3b99fc;
  }

  .knob-marker {
    width: 1.5px;
    height: 7px;
    background-color: #ffffff;
    position: absolute;
    top: 1.5px;
    left: calc(50% - 0.75px);
    transform-origin: bottom center;
    border-radius: 1px;
  }

  .knob-value {
    font-size: 0.65rem;
    font-family: monospace;
    color: #ffffff;
    text-align: center;
  }

  /* Active vs Muted knobs coloring */
  .active-knobs .knob-circle {
    border-color: #444444;
  }
  .active-knobs .knob-marker {
    background-color: #3b99fc; /* active highlight */
  }
  .active-knobs .knob-value {
    color: #3b99fc;
  }

  .placeholder-knobs .knob-circle {
    border-color: #333333;
    opacity: 0.6;
    cursor: not-allowed;
  }
  .placeholder-knobs .knob-marker {
    background-color: #717171;
  }
  .placeholder-knobs .knob-value {
    color: #717171;
  }

  /* Context Menu layout */
  .context-menu {
    position: fixed;
    background-color: #2a2a2b;
    border: 1px solid #444445;
    border-radius: 4px;
    box-shadow: 0 4px 12px rgba(0,0,0,0.5);
    z-index: 10000;
    min-width: 185px;
    padding: 4px 0;
  }

  .menu-item {
    padding: 8px 14px;
    font-size: 0.8rem;
    color: #d1d1d1;
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  .menu-item:hover {
    background-color: #3b99fc;
    color: #ffffff;
  }
</style>
