<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import * as pdfjsLib from "pdfjs-dist";
  import pdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

  pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorker;
  const openDialog = open;

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

  // Associated Versions & PDF Chart State
  let associatedVersions: PlaylistItem[] = [];
  let pdfChartPath = "";
  let pdfChartName = "";
  let pdfContainer: HTMLDivElement;
  let isLoadingPdf = false;
  let pdfTotalPages = 0;
  let pdfCurrentPage = 1;
  let isPdfInverted = false;
  let pdfRenderError = "";
  let currentRenderTaskId = 0;
  let lastRenderedWidth = 0;

  // Track Models
  interface Track {
    name: string;
    path: string;
    duration: number;
    sampleRate: number;
    channels: number;
    overviewPeaks: number[];
    pyramidPeaks: number[];
  }

  let mainTrack: Track | null = null;
  let alternateTrack: Track | null = null;
  let activeTrackMode: "main" | "alternate" = "main";

  // Left Panel tabs
  let activeTab: "browser" | "playlist" = "browser";

  // Rehearsal Workstation Knob State (Double-click resets)
  let speed = 1.0;                  // range 0.25 - 4.0 (25% - 400%)
  let pitch = 0;                    // range -24 - +24 semitones (in the middle)
  let pitchCents = 0;               // range -100 - +100 cents (fine tune)
  let eqBass = 0.0;                 // Low Shelf: 100 Hz (-12 - +12 dB)
  let eqMid = 0.0;                  // Mid Parametric Bell: 1 kHz (-12 - +12 dB)
  let eqTreble = 0.0;               // High Shelf: 8 kHz (-12 - +12 dB)
  let compressorThreshold = 0.0;    // range -60 - 0 dB (default 0 dB)
  let compressorRatio = 1.0;        // range 1.0 - 4.0 (default 1.0:1)
  let compressorMakeup = 0.0;       // range 0 - 24 dB (default 0.0 dB)
  
  // Regions System State (Milestones 3 & 6)
  interface Region {
    id: string;
    name: string;
    startTime: number;
    endTime: number;
    isLoop: boolean;
    isCut: boolean;
    color?: string;
    crossfadeMs?: number;
  }
  let regions: Region[] = [];
  let nextRegionId = 1;
  let selectedRegionId: string | null = null;
  let selectedMarkerIds: Set<string | number> = new Set();
  let timeSelection: { start: number; end: number } | null = null;
  let isShiftSelecting = false;
  let selectionDragStart = 0;
  let editingRegionId: string | null = null;
  let editingRegionName = "";

  // Region Context Menu
  let showRegionContextMenu = false;
  let regionContextMenuX = 0;
  let regionContextMenuY = 0;
  let contextMenuRegion: Region | null = null;

  // Advanced DSP Modals (Side 90° tab buttons)
  let showAdvancedCompModal = false;
  let showAdvancedEqModal = false;

  // Audio Export Modal State
  let showExportModal = false;
  let exportBitDepth: "int16" | "int24" | "float32" = "int24";
  let exportRange: "full" | "selection" | "region" = "full";
  let exportSelectedRegionId: string | null = null;
  let exportBakePitch = true;
  let exportBakeSpeed = true;
  let exportBakeEq = true;
  let exportBakeCompressor = true;
  let exportBakeCuts = true;
  let exportCopyMetadata = true;
  let isExporting = false;
  let exportStatusMessage = "";
  let exportErrorMessage = "";

  // Show Control & Remotes State (Milestone 8)
  let showRemoteSettingsModal = false;
  let midiPorts: string[] = [];
  let selectedMidiPort = "";
  let midiStatusMessage = "";
  
  // Center Lower Deck tabs & Metadata
  let activeCenterTab: "notes" | "lyrics" | "metadata" | "pdf" = "notes";
  let songNotes = "";
  let songLyrics = "";

  interface AudioTagMeta {
    title?: string;
    artist?: string;
    album?: string;
    grouping?: string;
    composer?: string;
    genre?: string;
    year?: number;
    track_number?: number;
    comment?: string;
    is_editable?: boolean;
  }
  let audioTags: AudioTagMeta = {};
  let isSavingTags = false;
  let tagSaveFeedback = "";
  
  // Slider State (Dbl-click resets)
  let dbVolume = 0.0;               // range -60 - +12 dB
  let zoom = 1.0;
  let zoomSliderVal = 0;            // 0 - 1000 logarithmic scale
  
  let totalFrames = 1;
  let maxZoom = 1.0;
  let target15sZoom = 1.0;

  $: totalFrames = duration > 0 && sampleRate > 0 ? duration * sampleRate : 1;
  $: maxZoom = Math.max(1.0, totalFrames / 9.0); // Exactly 9 samples on screen at max zoom!
  $: target15sZoom = duration > 0 ? Math.max(1.0, duration / 15.0) : 1.0;

  // Zoom View State (Continuous Single Line + Synchronous Local Cache)
  interface SampleCache {
    startFrame: number;
    endFrame: number;
    samples: number[];
  }
  let localSampleCache: SampleCache = { startFrame: -1, endFrame: -1, samples: [] };
  let isFetchingRawChunk = false;
  let currentRawSampleReqId = 0;
  let isWaveformDirty = true;

  let visibleSamples: number[] = [];
  let visibleSampleFrames = 0;

  // Markers & Regions
  interface Marker {
    id: number;
    name: string;
    time: number;
    color?: string;
    pdfAnchor?: {
      page: number;
      xPct: number;
      yPct: number;
    } | null;
  }
  let markers: Marker[] = [];
  let nextMarkerId = 1;
  let isDraggingMarker = false;
  let draggingMarkerId: number | null = null;
  let currentlyDraggedMarkerId: number | null = null;
  let isDraggingBadgeFromPdf = false;
  let lastScrolledMarkerId: number | null = null;

  const MARKER_COLORS = [
    "#ff9500", // Amber / Orange
    "#3b99fc", // Blue / Cyan
    "#34c759", // Green
    "#af52de", // Purple
    "#ff3b30", // Red
    "#ffd60a", // Yellow
    "#ff2d55", // Magenta / Pink
    "#00c7be", // Teal
    "#5856d6", // Indigo
    "#ffffff", // White
  ];

  let colorPaletteMarker: Marker | null = null;
  let colorPaletteX = 0;
  let colorPaletteY = 0;

  let editingMarkerId: number | null = null;
  let editingMarkerName = "";

  // Shared Project Landmarks Pool across all versions (Main, Alt, and Associated)
  let projectUnplacedLandmarks: Marker[] = [];
  $: projectUnplacedLandmarks = (() => {
    // Include current markers, mainTrack, alternateTrack, filePath as dependencies
    const _dep = [markers, activeTrackMode, filePath, mainTrack, alternateTrack, associatedVersions];
    const store = getProfilesStore();
    const allKnownMarkers: Marker[] = [];

    // 1. Check Main track
    if (mainTrack && store[mainTrack.path]?.markers) {
      for (const m of store[mainTrack.path].markers) {
        if (!allKnownMarkers.some(k => k.name.trim().toLowerCase() === m.name.trim().toLowerCase())) {
          allKnownMarkers.push(m);
        }
      }
    }
    // 2. Check Alternate track
    if (alternateTrack && store[alternateTrack.path]?.markers) {
      for (const m of store[alternateTrack.path].markers) {
        if (!allKnownMarkers.some(k => k.name.trim().toLowerCase() === m.name.trim().toLowerCase())) {
          allKnownMarkers.push(m);
        }
      }
    }
    // 3. Check Associated versions
    for (const v of associatedVersions) {
      if (store[v.path]?.markers) {
        for (const m of store[v.path].markers) {
          if (!allKnownMarkers.some(k => k.name.trim().toLowerCase() === m.name.trim().toLowerCase())) {
            allKnownMarkers.push(m);
          }
        }
      }
    }

    // Filter out markers already placed on the current active track
    return allKnownMarkers.filter(pm => 
      !markers.some(m => m.name.trim().toLowerCase() === pm.name.trim().toLowerCase())
    );
  })();

  // OS Folder Browser State
  let currentPath = "";
  let parentPath: string | null = null;
  let browserEntries: any[] = [];
  let cloudFolders: PlaylistItem[] = [];
  
  // Search & Type to Jump State
  let searchQuery = "";
  let filteredEntries: any[] = [];
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
  let contextMenuType: "browser" | "waveform" = "browser";
  let contextMenuX = 0;
  let contextMenuY = 0;
  let contextMenuTargetFile: { name: string; path: string } | null = null;

  // Playlist State (saved in localStorage)
  interface PlaylistItem {
    name: string;
    path: string;
  }
  let playlistItems: PlaylistItem[] = [];
  let selectedPlaylistIndex = -1;

  // Project Setup: File Associations (saved in localStorage)
  // Associated Files Management
  interface AssociatedFileItem {
    id: string;
    name: string;
    path: string;
    fileType: "pdf" | "audio" | "other";
  }
  let associatedFiles: AssociatedFileItem[] = [];

  // Dynamic Open PDF Tabs
  interface OpenPdfTab {
    id: string;
    name: string;
    path: string;
    totalPages: number;
    currentPage: number;
    isLoading: boolean;
    error: string | null;
    isInverted: boolean;
  }
  let openPdfTabs: OpenPdfTab[] = [];
  let activePdfTabId: string | null = null;

  // Notes & Lyrics View Modes (edit, preview, split)
  let notesViewMode: "edit" | "preview" | "split" = "edit";
  let lyricsViewMode: "edit" | "preview" | "split" = "edit";

  function renderMarkdown(md: string): string {
    if (!md || !md.trim()) return "<div class='markdown-empty-hint'>No notes recorded yet. Click <strong>Edit</strong> or <strong>Split</strong> to add notes or chord charts...</div>";
    
    let html = md
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

    // Highlight chords in brackets e.g. [C#m7], [G/B], [F#7b9]
    html = html.replace(/\[([A-G][b#]?(?:m|maj|min|dim|aug|sus|add)?[0-9]?(?:[b#][0-9])?(?:\/[A-G][b#]?)?)\]/g, '<span class="chord-badge">$1</span>');

    // Headings
    html = html.replace(/^### (.*$)/gim, '<h3 class="md-h3">$1</h3>');
    html = html.replace(/^## (.*$)/gim, '<h2 class="md-h2">$1</h2>');
    html = html.replace(/^# (.*$)/gim, '<h1 class="md-h1">$1</h1>');

    // Blockquotes
    html = html.replace(/^\> (.*$)/gim, '<blockquote class="md-quote">$1</blockquote>');

    // Bold & Italics
    html = html.replace(/\*\*\*(.*?)\*\*\*/gim, '<strong><em>$1</em></strong>');
    html = html.replace(/\*\*(.*?)\*\*/gim, '<strong>$1</strong>');
    html = html.replace(/\*(.*?)\*/gim, '<em>$1</em>');
    html = html.replace(/~~(.*?)~~/gim, '<del>$1</del>');

    // Inline code / chord lines
    html = html.replace(/`([^`]+)`/gim, '<code class="md-code">$1</code>');

    // Lists
    html = html.replace(/^\s*-\s+(.*$)/gim, '<li class="md-li">$1</li>');

    // Paragraphs and newlines
    html = html.replace(/\n\n+/g, '</p><p class="md-p">');
    html = html.replace(/\n/g, '<br/>');

    return `<div class="markdown-rendered-body"><p class="md-p">${html}</p></div>`;
  }

  // DSP Effects Bypass States
  let isEqBypassed = false;
  let isCompressorBypassed = false;

  // Multi-Node EQ State (Kirchhoff / AnyTune Inspired)
  interface EqNodeState {
    id: string;
    name: string;
    filterType: "LowShelf" | "Peaking" | "HighShelf" | "LowPass" | "HighPass" | "Notch";
    freq: number;
    gainDb: number;
    q: number;
    enabled: boolean;
    color: string;
  }

  let eqNodes: EqNodeState[] = [
    { id: "node-1", name: "Low Shelf", filterType: "LowShelf", freq: 100, gainDb: 0, q: 0.707, enabled: true, color: "#3b99fc" },
    { id: "node-2", name: "Mid Bell", filterType: "Peaking", freq: 1000, gainDb: 0, q: 1.0, enabled: true, color: "#30d158" },
    { id: "node-3", name: "High Shelf", filterType: "HighShelf", freq: 8000, gainDb: 0, q: 0.707, enabled: true, color: "#ff9500" },
  ];
  let selectedEqNodeId: string = "node-2";
  let selEqNode: EqNodeState = eqNodes[1];
  $: selEqNode = eqNodes.find(n => n.id === selectedEqNodeId) || eqNodes[0];
  let showEqFilterMenu = false;
  let eqFilterMenuX = 0;
  let eqFilterMenuY = 0;
  let eqFilterMenuTargetNode: EqNodeState | null = null;

  function getEqPath(nodes: EqNodeState[]): string {
    if (!nodes || nodes.length === 0) return "M 20 120 L 580 120";
    let d = "M 20 120";
    const sorted = [...nodes].sort((a, b) => a.freq - b.freq);
    for (const node of sorted) {
      const nx = 20 + ((Math.log10(Math.max(20, Math.min(20000, node.freq))) - 1.30103) / 3.0) * 560;
      const ny = node.enabled ? 120 - Math.max(-24, Math.min(24, node.gainDb)) * (100 / 24) : 120;
      d += ` S ${nx - 20} ${ny} ${nx} ${ny}`;
    }
    d += " L 580 120";
    return d;
  }

  // Dual Stage Compressor State (Sonitus Inspired)
  interface CompStageState {
    enabled: boolean;
    compType: "Vintage" | "Modern" | "FET" | "Opto";
    thresholdDb: number;
    ratio: number;
    kneeDb: number;
    attackMs: number;
    releaseMs: number;
    makeupDb: number;
    limiter: boolean;
  }

  let compStage1: CompStageState = {
    enabled: true,
    compType: "Vintage",
    thresholdDb: 0.0,
    ratio: 1.0,
    kneeDb: 3.0,
    attackMs: 30.0,
    releaseMs: 300.0,
    makeupDb: 0.0,
    limiter: false,
  };

  let compStage2: CompStageState = {
    enabled: false,
    compType: "Opto",
    thresholdDb: -12.0,
    ratio: 2.0,
    kneeDb: 4.0,
    attackMs: 50.0,
    releaseMs: 500.0,
    makeupDb: 0.0,
    limiter: false,
  };

  let compRouting: "Series" | "Parallel" = "Series";
  let compParallelBlend: number = 0.5;
  let activeCompStageTab: 1 | 2 = 1;
  let curCompStage: CompStageState = compStage1;
  $: curCompStage = activeCompStageTab === 1 ? compStage1 : compStage2;

  function getSonitusCurvePath(stage: CompStageState): string {
    if (!stage) return "M 20 180 L 280 20";
    const threshPxX = 20 + (60 + stage.thresholdDb) * (260 / 60);
    const threshPxY = 180 - (60 + stage.thresholdDb + stage.makeupDb) * (160 / 60);
    const kneeHalfX = (stage.kneeDb / 2) * (260 / 60);
    const kneeHalfY = (stage.kneeDb / 2) * (160 / 60);
    const endOutDb = stage.thresholdDb + ((0 - stage.thresholdDb) / stage.ratio) + stage.makeupDb;
    const endPxY = 180 - (60 + endOutDb) * (160 / 60);

    if (stage.kneeDb > 0.5) {
      return `M 20 ${180 - stage.makeupDb * (160 / 60)} L ${Math.max(20, threshPxX - kneeHalfX)} ${Math.min(180, threshPxY + kneeHalfY)} Q ${threshPxX} ${threshPxY} ${Math.min(280, threshPxX + kneeHalfX)} ${threshPxY - kneeHalfY / stage.ratio} L 280 ${endPxY}`;
    } else {
      return `M 20 ${180 - stage.makeupDb * (160 / 60)} L ${threshPxX} ${threshPxY} L 280 ${endPxY}`;
    }
  }

  function getSonitusSignalDot(stage: CompStageState, inDb: number): { x: number, y: number } {
    if (!stage) return { x: 20, y: 180 };
    const liveDotX = 20 + (60 + Math.max(-60, Math.min(0, inDb))) * (260 / 60);
    const liveDotOutDb = inDb <= stage.thresholdDb ? inDb + stage.makeupDb : stage.thresholdDb + ((inDb - stage.thresholdDb) / stage.ratio) + stage.makeupDb;
    const liveDotY = 180 - (60 + Math.max(-60, Math.min(6, liveDotOutDb))) * (160 / 60);
    return { x: liveDotX, y: liveDotY };
  }

  // Real-time Visual Meter Values
  let liveInputPeakL: number = -60.0;
  let liveInputPeakR: number = -60.0;
  let liveGainReductionDb: number = 0.0;
  let liveOutputPeakL: number = -60.0;
  let liveOutputPeakR: number = -60.0;

  function switchCenterTab(tab: string) {
    activeCenterTab = tab;
    lastScrolledMarkerId = null;
    localStorage.setItem("th_last_center_tab", tab);
    saveCurrentTrackProfile(filePath);
    if (tab.startsWith("pdf-")) {
      activePdfTabId = tab.replace("pdf-", "");
      tick().then(() => renderPdfMarkerBadges());
    }
  }

  function toggleDynamicTabInvert(tabId: string) {
    const tab = openPdfTabs.find(t => t.id === tabId);
    if (tab) {
      tab.isInverted = !tab.isInverted;
      openPdfTabs = [...openPdfTabs];
      renderOpenPdfTab(tab);
    }
  }

  function openPdfTab(path: string, name?: string) {
    if (!path) return;
    const docName = name || (path.split("/").pop() || "Document.pdf");
    let existing = openPdfTabs.find(t => t.path === path);
    if (!existing) {
      const newTab: OpenPdfTab = {
        id: "pdf_" + Date.now() + "_" + Math.random().toString(36).substring(2, 6),
        name: docName,
        path: path,
        totalPages: 0,
        currentPage: 1,
        isLoading: true,
        error: null,
        isInverted: false
      };
      openPdfTabs = [...openPdfTabs, newTab];
      existing = newTab;
    }
    activePdfTabId = existing.id;
    switchCenterTab("pdf-" + existing.id);
    tick().then(() => renderOpenPdfTab(existing!));
  }

  function closePdfTab(id: string) {
    const tabIndex = openPdfTabs.findIndex(t => t.id === id);
    openPdfTabs = openPdfTabs.filter(t => t.id !== id);
    if (activeCenterTab === "pdf-" + id) {
      if (openPdfTabs.length > 0) {
        const nextTab = openPdfTabs[Math.max(0, tabIndex - 1)];
        switchCenterTab("pdf-" + nextTab.id);
      } else {
        switchCenterTab("files");
      }
    }
  }

  // Per-Track Persistent Project Profiles (AnyTune style)
  interface TrackProfile {
    dbVolume: number;
    speed: number;
    pitch: number;
    pitchCents?: number;
    eqBass: number;
    eqMid: number;
    eqTreble: number;
    isEqBypassed?: boolean;
    eqNodes?: EqNodeState[];
    compressorThreshold: number;
    compressorRatio: number;
    compressorMakeup: number;
    isCompressorBypassed?: boolean;
    compStage1?: CompStageState;
    compStage2?: CompStageState;
    compRouting?: "Series" | "Parallel";
    compParallelBlend?: number;
    markers: Marker[];
    nextMarkerId: number;
    regions?: Region[];
    nextRegionId?: number;
    associatedFiles?: AssociatedFileItem[];
    pdfChartPath?: string;
    pdfChartName?: string;
    associatedVersions?: PlaylistItem[];
    alternateTrackPath?: string | null;
    notes?: string;
    lyrics?: string;
    notesViewMode?: "edit" | "preview" | "split";
    lyricsViewMode?: "edit" | "preview" | "split";
    lastCenterTab?: string;
  }

  let cachedProfilesStore: Record<string, TrackProfile> | null = null;
  let profileSaveDebounceTimer: any = null;

  function getProfilesStore(): Record<string, TrackProfile> {
    if (cachedProfilesStore) return cachedProfilesStore;
    try {
      const data = localStorage.getItem("th_track_profiles");
      cachedProfilesStore = data ? JSON.parse(data) : {};
    } catch {
      cachedProfilesStore = {};
    }
    return cachedProfilesStore;
  }

  function flushProfilesToLocalStorage() {
    if (profileSaveDebounceTimer) {
      clearTimeout(profileSaveDebounceTimer);
      profileSaveDebounceTimer = null;
    }
    if (cachedProfilesStore) {
      try {
        localStorage.setItem("th_track_profiles", JSON.stringify(cachedProfilesStore));
      } catch (e) {
        console.error("Failed to save track profiles to localStorage:", e);
      }
    }
  }

  function saveCurrentTrackProfile(trackPath: string | null, immediate = false) {
    if (!trackPath) return;
    const store = getProfilesStore();
    const existing = store[trackPath] || ({} as TrackProfile);
    const isMain = activeTrackMode === "main" || trackPath === mainTrack?.path;

    store[trackPath] = {
      ...existing,
      dbVolume,
      speed,
      pitch,
      pitchCents,
      eqBass,
      eqMid,
      eqTreble,
      isEqBypassed,
      eqNodes: eqNodes.map(n => ({ ...n })),
      compressorThreshold: compStage1.thresholdDb,
      compressorRatio: compStage1.ratio,
      compressorMakeup: compStage1.makeupDb,
      isCompressorBypassed,
      compStage1: { ...compStage1 },
      compStage2: { ...compStage2 },
      compRouting,
      compParallelBlend,
      markers: markers.map(m => ({ ...m })),
      nextMarkerId,
      regions: regions.map(r => ({ ...r })),
      nextRegionId,
      associatedFiles: isMain ? associatedFiles.map(a => ({ ...a })) : (existing.associatedFiles || []),
      pdfChartPath: isMain ? pdfChartPath : (existing.pdfChartPath || ""),
      pdfChartName: isMain ? pdfChartName : (existing.pdfChartName || ""),
      associatedVersions: isMain ? associatedVersions.map(v => ({ ...v })) : (existing.associatedVersions || []),
      alternateTrackPath: isMain ? (alternateTrack ? alternateTrack.path : null) : (existing.alternateTrackPath || null),
      notes: isMain ? songNotes : (existing.notes || ""),
      lyrics: isMain ? songLyrics : (existing.lyrics || ""),
      notesViewMode,
      lyricsViewMode,
      lastCenterTab: activeCenterTab
    };

    if (immediate) {
      flushProfilesToLocalStorage();
    } else {
      if (profileSaveDebounceTimer) clearTimeout(profileSaveDebounceTimer);
      profileSaveDebounceTimer = setTimeout(flushProfilesToLocalStorage, 400);
    }
  }

  async function updatePitchEngine() {
    const totalSemitones = pitch + (pitchCents / 100.0);
    await invoke("set_pitch", { pitch: totalSemitones });
    saveCurrentTrackProfile(filePath);
  }

  async function updateEqEngine() {
    if (isEqBypassed) {
      await invoke("set_eq_bands", { bands: [] });
    } else if (eqNodes && eqNodes.length > 0) {
      await invoke("set_eq_bands", {
        bands: eqNodes.map(n => ({
          filterType: n.filterType,
          freq: n.freq,
          gainDb: n.enabled ? n.gainDb : 0.0,
          q: n.q,
          enabled: n.enabled
        }))
      });
    } else {
      await invoke("set_eq", { bassDb: eqBass, midDb: eqMid, trebleDb: eqTreble });
    }
    saveCurrentTrackProfile(filePath);
  }

  async function updateCompressorEngine() {
    compressorThreshold = compStage1.thresholdDb;
    compressorRatio = compStage1.ratio;
    compressorMakeup = compStage1.makeupDb;

    if (isCompressorBypassed) {
      await invoke("set_compressor", {
        thresholdDb: 0.0,
        ratio: 1.0,
        makeupDb: 0.0,
        attackMs: 30.0,
        releaseMs: 300.0
      });
    } else {
      await invoke("set_dual_compressor", {
        stage1: {
          enabled: compStage1.enabled,
          compType: compStage1.compType,
          thresholdDb: compStage1.thresholdDb,
          ratio: compStage1.ratio,
          kneeDb: compStage1.kneeDb,
          attackMs: compStage1.attackMs,
          releaseMs: compStage1.releaseMs,
          makeupDb: compStage1.makeupDb
        },
        stage2: {
          enabled: compStage2.enabled,
          compType: compStage2.compType,
          thresholdDb: compStage2.thresholdDb,
          ratio: compStage2.ratio,
          kneeDb: compStage2.kneeDb,
          attackMs: compStage2.attackMs,
          releaseMs: compStage2.releaseMs,
          makeupDb: compStage2.makeupDb
        },
        routing: compRouting,
        parallelBlend: compParallelBlend
      });
    }
    saveCurrentTrackProfile(filePath);
    drawMainWaveform();
  }

  async function syncRegionsToEngine() {
    try {
      await invoke("set_regions", {
        regions: regions.map(r => ({
          startSeconds: r.startTime,
          endSeconds: r.endTime,
          isLoop: r.isLoop,
          isCut: r.isCut,
          crossfadeMs: r.crossfadeMs ?? 5.0
        }))
      });
      saveCurrentTrackProfile(filePath);
      invalidateWaveformCaches();
      drawMainWaveform();
    } catch (e) {
      console.error("Failed to sync regions to audio engine:", e);
    }
  }

  async function saveAudioTags() {
    if (!filePath) return;
    isSavingTags = true;
    tagSaveFeedback = "";
    try {
      await invoke("save_audio_metadata", { path: filePath, metadata: audioTags });
      tagSaveFeedback = "✓ Saved tags to audio file!";
      setTimeout(() => tagSaveFeedback = "", 3500);
      // If title changed, update displayed title
      if (audioTags.title && mainTrack && activeTrackMode === "main") {
        fileName = audioTags.title;
      }
    } catch (err) {
      alert("Failed to save audio tags: " + err);
    } finally {
      isSavingTags = false;
    }
  }

  // Associated Files Management Methods
  async function addAssociatedFilePicker() {
    try {
      const selected = await openDialog({
        multiple: true,
        filters: [{
          name: "All Associated Rehearsal Media",
          extensions: ["pdf", "wav", "mp3", "flac", "m4a", "aac", "ogg", "aiff", "aif"]
        }, {
          name: "PDF Sheet Music & Charts",
          extensions: ["pdf"]
        }, {
          name: "Audio Tracks & Stems",
          extensions: ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aiff", "aif"]
        }]
      });

      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      for (const p of paths) {
        if (!p) continue;
        const ext = p.split(".").pop()?.toLowerCase() || "";
        const fName = p.split("/").pop() || p;
        const isPdf = ext === "pdf";
        const isAudio = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aiff", "aif"].includes(ext);

        if (!associatedFiles.some(f => f.path === p)) {
          associatedFiles.push({
            id: "file_" + Date.now() + "_" + Math.random().toString(36).substring(2, 6),
            name: fName,
            path: p,
            fileType: isPdf ? "pdf" : isAudio ? "audio" : "other"
          });
        }
      }
      associatedFiles = [...associatedFiles];
      saveCurrentTrackProfile(filePath);
    } catch (e) {
      console.error("Failed to add associated file:", e);
    }
  }

  function unlinkAssociatedFile(id: string) {
    associatedFiles = associatedFiles.filter(f => f.id !== id);
    saveCurrentTrackProfile(filePath);
  }

  async function renderOpenPdfTab(tab: OpenPdfTab) {
    const container = document.getElementById("pdf-container-" + tab.id) as HTMLDivElement | null;
    if (!container) return;
    tab.isLoading = true;
    tab.error = null;
    openPdfTabs = [...openPdfTabs];

    try {
      const bytes: number[] = await invoke("read_file_bytes", { path: tab.path });
      const uint8 = new Uint8Array(bytes);
      const loadingTask = pdfjsLib.getDocument({ data: uint8 });
      const doc = await loadingTask.promise;
      
      tab.totalPages = doc.numPages;
      container.innerHTML = "";
      const containerWidth = Math.max(300, (container.clientWidth || 800) - 32);
      const dpr = window.devicePixelRatio || 1;

      for (let pageNum = 1; pageNum <= doc.numPages; pageNum++) {
        const page = await doc.getPage(pageNum);
        const unscaledViewport = page.getViewport({ scale: 1.0 });
        const baseScale = containerWidth / unscaledViewport.width;
        const viewport = page.getViewport({ scale: baseScale * dpr });

        const pageWrapper = document.createElement("div");
        pageWrapper.className = `pdf-page-card ${tab.isInverted ? 'inverted' : ''}`;
        pageWrapper.dataset.pageNum = pageNum.toString();

        const canvas = document.createElement("canvas");
        canvas.className = "pdf-page-canvas";
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        canvas.style.width = `${Math.floor(viewport.width / dpr)}px`;
        canvas.style.height = `${Math.floor(viewport.height / dpr)}px`;

        const ctx = canvas.getContext("2d")!;
        pageWrapper.appendChild(canvas);
        container.appendChild(pageWrapper);

        await page.render({ canvasContext: ctx, viewport }).promise;
      }

      tab.isLoading = false;
      openPdfTabs = [...openPdfTabs];
      renderPdfMarkerBadges();
    } catch (err: any) {
      tab.isLoading = false;
      tab.error = "Failed to render PDF: " + (err.message || err);
      openPdfTabs = [...openPdfTabs];
    }
  }

  function handleDynamicPdfScroll(e: Event, tab: OpenPdfTab) {
    const container = e.target as HTMLElement;
    if (!container) return;
    const cards = container.querySelectorAll(".pdf-page-card");
    const containerTop = container.getBoundingClientRect().top;
    const containerMid = containerTop + (container.clientHeight * 0.35);

    for (let i = 0; i < cards.length; i++) {
      const card = cards[i] as HTMLElement;
      const rect = card.getBoundingClientRect();
      if (rect.top <= containerMid && rect.bottom >= containerMid) {
        const p = parseInt(card.dataset.pageNum || "1", 10);
        if (p !== tab.currentPage) {
          tab.currentPage = p;
          openPdfTabs = [...openPdfTabs];
        }
        break;
      }
    }
  }

  function handleDynamicPdfContainerDrop(e: DragEvent, tab: OpenPdfTab) {
    e.preventDefault();
    e.stopPropagation();
  }

  function handlePdfScroll() {
    if (!pdfContainer) return;
    const cards = pdfContainer.querySelectorAll(".pdf-page-card");
    const containerTop = pdfContainer.getBoundingClientRect().top;
    const containerMid = containerTop + (pdfContainer.clientHeight * 0.35);

    for (let i = 0; i < cards.length; i++) {
      const card = cards[i] as HTMLElement;
      const rect = card.getBoundingClientRect();
      if (rect.top <= containerMid && rect.bottom >= containerMid) {
        const p = parseInt(card.dataset.pageNum || "1", 10);
        if (p !== pdfCurrentPage) {
          pdfCurrentPage = p;
        }
        break;
      }
    }
  }

  async function renderPdfPages() {
    if (!pdfChartPath || !pdfContainer) return;
    const taskId = ++currentRenderTaskId;
    isLoadingPdf = true;
    pdfRenderError = "";
    pdfCurrentPage = 1;

    try {
      const bytes: number[] = await invoke("read_file_bytes", { path: pdfChartPath });
      if (taskId !== currentRenderTaskId) return;
      const uint8 = new Uint8Array(bytes);
      const loadingTask = pdfjsLib.getDocument({ data: uint8 });
      const doc = await loadingTask.promise;
      if (taskId !== currentRenderTaskId) return;

      pdfTotalPages = doc.numPages;
      pdfContainer.innerHTML = "";

      const containerWidth = Math.max(300, (pdfContainer.clientWidth || 800) - 32);
      lastRenderedWidth = containerWidth;
      const dpr = window.devicePixelRatio || 1;

      for (let pageNum = 1; pageNum <= doc.numPages; pageNum++) {
        if (taskId !== currentRenderTaskId) return;
        const page = await doc.getPage(pageNum);
        const unscaledViewport = page.getViewport({ scale: 1.0 });
        
        // Auto-scale to full container width
        const baseScale = containerWidth / unscaledViewport.width;
        const viewport = page.getViewport({ scale: baseScale * dpr });

        const pageWrapper = document.createElement("div");
        pageWrapper.className = `pdf-page-card ${isPdfInverted ? 'inverted' : ''}`;
        pageWrapper.dataset.pageNum = pageNum.toString();

        pageWrapper.addEventListener("dragover", (e) => {
          e.preventDefault();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
        });
        pageWrapper.addEventListener("drop", (e) => {
          e.preventDefault();
          e.stopPropagation();
          handlePdfPageDrop(e, pageNum, pageWrapper);
        });

        const canvas = document.createElement("canvas");
        canvas.className = "pdf-page-canvas";
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        canvas.style.width = `${Math.floor(viewport.width / dpr)}px`;
        canvas.style.height = `${Math.floor(viewport.height / dpr)}px`;

        const ctx = canvas.getContext("2d")!;

        pageWrapper.appendChild(canvas);
        pdfContainer.appendChild(pageWrapper);

        const renderContext = {
          canvasContext: ctx,
          viewport: viewport
        };
        await page.render(renderContext).promise;
      }

      renderPdfMarkerBadges();
    } catch (err: any) {
      if (taskId === currentRenderTaskId) {
        console.error("PDF render error:", err);
        pdfRenderError = "Failed to render PDF: " + (err.message || err);
      }
    } finally {
      if (taskId === currentRenderTaskId) {
        isLoadingPdf = false;
      }
    }
  }

  function handlePdfPageDrop(e: DragEvent, pageNum: number, pageWrapper: HTMLElement) {
    e.preventDefault();
    e.stopPropagation();

    const markerIdStr = e.dataTransfer?.getData("text/trackhelm-marker-id") || e.dataTransfer?.getData("text/plain");
    const markerId = currentlyDraggedMarkerId || (markerIdStr ? parseInt(markerIdStr, 10) : null);
    if (!markerId) return;
    const marker = markers.find(m => m.id === markerId);
    if (!marker) return;

    const rect = pageWrapper.getBoundingClientRect();
    const xPct = Math.max(0.01, Math.min(0.95, (e.clientX - rect.left) / rect.width));
    const yPct = Math.max(0.01, Math.min(0.98, (e.clientY - rect.top) / rect.height));

    marker.pdfAnchor = {
      page: pageNum,
      xPct,
      yPct
    };

    currentlyDraggedMarkerId = null;
    isDraggingBadgeFromPdf = false;

    markers = markers;
    saveCurrentTrackProfile(filePath);
    renderPdfMarkerBadges();
  }

  function handlePdfContainerDrop(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();

    const markerIdStr = e.dataTransfer?.getData("text/trackhelm-marker-id") || e.dataTransfer?.getData("text/plain");
    const markerId = currentlyDraggedMarkerId || (markerIdStr ? parseInt(markerIdStr, 10) : null);
    if (!markerId) return;

    const marker = markers.find(m => m.id === markerId);
    if (!marker || !pdfContainer) return;

    const cards = pdfContainer.querySelectorAll(".pdf-page-card");
    let droppedOnCard = false;

    for (let i = 0; i < cards.length; i++) {
      const card = cards[i] as HTMLElement;
      const rect = card.getBoundingClientRect();
      if (
        e.clientX >= rect.left &&
        e.clientX <= rect.right &&
        e.clientY >= rect.top &&
        e.clientY <= rect.bottom
      ) {
        const pageNum = parseInt(card.dataset.pageNum || "1", 10);
        const xPct = Math.max(0.01, Math.min(0.95, (e.clientX - rect.left) / rect.width));
        const yPct = Math.max(0.01, Math.min(0.98, (e.clientY - rect.top) / rect.height));

        marker.pdfAnchor = {
          page: pageNum,
          xPct,
          yPct
        };
        droppedOnCard = true;
        break;
      }
    }

    if (!droppedOnCard && isDraggingBadgeFromPdf) {
      marker.pdfAnchor = null;
    }

    currentlyDraggedMarkerId = null;
    isDraggingBadgeFromPdf = false;

    markers = markers;
    saveCurrentTrackProfile(filePath);
    renderPdfMarkerBadges();
  }

  function removeMarkerPdfAnchor(markerId: number) {
    const marker = markers.find(m => m.id === markerId);
    if (marker) {
      marker.pdfAnchor = null;
      markers = markers;
      saveCurrentTrackProfile(filePath);
      renderPdfMarkerBadges();
    }
  }

  // Universal Pointer Drag Engine for Markers (Waveform, Sidebar & PDF Score)
  let activePointerDragMarker: Marker | null = null;
  let pointerDragGhost: HTMLDivElement | null = null;
  let pointerDragIsFromPdf = false;
  let pointerDragStartX = 0;
  let pointerDragStartY = 0;
  let hasPointerDragMoved = false;
  let dragWaveformPreviewMarker: Marker | null = null;
  let dragWaveformPreviewX: number | null = null;

  let pointerDragIsFromWaveform = false;

  function handleMarkerItemMouseDown(e: MouseEvent, marker: Marker) {
    const target = e.target as HTMLElement;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'BUTTON' || target.classList.contains('marker-color-dot') || target.classList.contains('delete-marker-btn') || target.classList.contains('marker-action-btn'))) {
      return;
    }
    if (editingMarkerId === marker.id) return;

    if (e.shiftKey) {
      if (selectedMarkerIds.has(marker.id)) {
        selectedMarkerIds.delete(marker.id);
      } else {
        if (selectedMarkerIds.size >= 2) selectedMarkerIds.clear();
        selectedMarkerIds.add(marker.id);
      }
      selectedMarkerIds = selectedMarkerIds;
      if (selectedMarkerIds.size === 2) {
        const arr = markers.filter(x => selectedMarkerIds.has(x.id)).sort((a, b) => a.time - b.time);
        if (arr.length === 2) {
          timeSelection = { start: arr[0].time, end: arr[1].time };
        }
      } else {
        timeSelection = null;
      }
      drawMainWaveform();
      return;
    }

    startMarkerPointerDrag(e, marker, false, false);
  }

  function startMarkerPointerDrag(e: MouseEvent, marker: Marker, isFromPdf = false, isFromWaveform = false) {
    if (e.button !== 0) return;
    activePointerDragMarker = marker;
    pointerDragIsFromPdf = isFromPdf;
    pointerDragIsFromWaveform = isFromWaveform;
    pointerDragStartX = e.clientX;
    pointerDragStartY = e.clientY;
    hasPointerDragMoved = false;
    dragWaveformPreviewMarker = isFromWaveform ? marker : null;
    dragWaveformPreviewX = isFromWaveform ? e.clientX : null;

    window.addEventListener("mousemove", handleMarkerPointerMouseMove);
    window.addEventListener("mouseup", handleMarkerPointerMouseUp);
  }

  function handleMarkerPointerMouseMove(e: MouseEvent) {
    if (!activePointerDragMarker) return;
    const dx = e.clientX - pointerDragStartX;
    const dy = e.clientY - pointerDragStartY;
    if (!hasPointerDragMoved && Math.hypot(dx, dy) > 4) {
      hasPointerDragMoved = true;
      if (!pointerDragGhost) {
        pointerDragGhost = document.createElement("div");
        pointerDragGhost.className = "marker-pointer-drag-ghost";
        pointerDragGhost.style.backgroundColor = activePointerDragMarker.color || "#ff9500";
        pointerDragGhost.innerHTML = `
          <span class="pdf-marker-dot"></span>
          <span>${activePointerDragMarker.name}</span>
        `;
        document.body.appendChild(pointerDragGhost);
      }
    }

    if (hasPointerDragMoved) {
      const elem = document.elementFromPoint(e.clientX, e.clientY);
      const isOverPdf = elem ? !!(elem.closest(".pdf-page-card") || elem.closest(".pdf-scroll-column")) : false;
      const isOverWaveform = elem ? !!(elem.closest(".block-main-waveform") || elem.closest("#mainCanvas")) : false;

      if (isOverWaveform && mainCanvas && duration > 0) {
        dragWaveformPreviewMarker = activePointerDragMarker;
        dragWaveformPreviewX = e.clientX;
        drawMainWaveform();
        if (pointerDragGhost) {
          pointerDragGhost.style.display = "none";
        }
      } else {
        if (dragWaveformPreviewMarker) {
          dragWaveformPreviewMarker = null;
          dragWaveformPreviewX = null;
          drawMainWaveform();
        }

        if (pointerDragGhost) {
          pointerDragGhost.style.display = "inline-flex";
          if (isOverPdf) {
            pointerDragGhost.className = "marker-pointer-drag-ghost on-pdf-preview";
          } else {
            pointerDragGhost.className = "marker-pointer-drag-ghost";
          }
          pointerDragGhost.style.left = `${e.clientX}px`;
          pointerDragGhost.style.top = `${e.clientY}px`;
        }
      }
    }
  }

  function handleMarkerPointerMouseUp(e: MouseEvent) {
    window.removeEventListener("mousemove", handleMarkerPointerMouseMove);
    window.removeEventListener("mouseup", handleMarkerPointerMouseUp);

    if (pointerDragGhost) {
      pointerDragGhost.remove();
      pointerDragGhost = null;
    }

    if (dragWaveformPreviewMarker) {
      dragWaveformPreviewMarker = null;
      dragWaveformPreviewX = null;
      drawMainWaveform();
    }

    if (!activePointerDragMarker) return;
    const marker = activePointerDragMarker;
    const isFromPdf = pointerDragIsFromPdf;
    const isFromWaveform = pointerDragIsFromWaveform;
    activePointerDragMarker = null;

    if (!hasPointerDragMoved) {
      return;
    }

    const elem = document.elementFromPoint(e.clientX, e.clientY);
    
    // 1. Check if dropped onto a PDF page card
    const pageCard = elem ? (elem.closest(".pdf-page-card") as HTMLElement | null) : null;
    if (pageCard) {
      const pageNum = parseInt(pageCard.dataset.pageNum || "1", 10);
      const rect = pageCard.getBoundingClientRect();
      const xPct = Math.max(0.01, Math.min(0.95, (e.clientX - rect.left) / rect.width));
      const yPct = Math.max(0.01, Math.min(0.98, (e.clientY - rect.top) / rect.height));

      const existing = markers.find(m => m.id === marker.id);
      if (existing) {
        existing.pdfAnchor = { page: pageNum, xPct, yPct };
      } else {
        markers = [...markers, { ...marker, id: nextMarkerId++, pdfAnchor: { page: pageNum, xPct, yPct } }];
      }
      markers = markers;
      saveCurrentTrackProfile(filePath);
      renderPdfMarkerBadges();
      return;
    }

    // 2. Check if dropped onto Main Waveform
    const waveformBlock = elem ? (elem.closest(".block-main-waveform") as HTMLElement | null) : null;
    if (waveformBlock && mainCanvas && duration > 0) {
      const rect = mainCanvas.getBoundingClientRect();
      const dropX = e.clientX - rect.left;
      const clickPct = Math.max(0, Math.min(1.0, dropX / rect.width));
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const dropTime = Math.max(0, Math.min(duration, (startProgress + clickPct * windowWidth) * duration));

      const existing = markers.find(m => m.id === marker.id);
      if (existing) {
        existing.time = dropTime;
      } else {
        markers = [...markers, { id: nextMarkerId++, name: marker.name, time: dropTime, color: marker.color || "#ff9500" }];
      }

      markers.sort((a, b) => a.time - b.time);
      markers = markers;
      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
      drawOverviewWaveform();
      if (activeCenterTab === "pdf") {
        renderPdfMarkerBadges();
      }
      return;
    }

    // 3. If dragged from PDF and dropped elsewhere outside PDF -> remove association
    if (isFromPdf) {
      marker.pdfAnchor = null;
      markers = markers;
      saveCurrentTrackProfile(filePath);
      renderPdfMarkerBadges();
      return;
    }

    // 4. If dragged from Waveform and dropped outside waveform and PDF -> unassign from this track!
    if (isFromWaveform) {
      markers = markers.filter(m => m.id !== marker.id);
      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
      drawOverviewWaveform();
      if (activeCenterTab === "pdf") {
        renderPdfMarkerBadges();
      }
      return;
    }
  }

  function renderPdfMarkerBadges() {
    if (!pdfContainer) return;
    const cards = pdfContainer.querySelectorAll(".pdf-page-card");
    cards.forEach(card => {
      const pageNum = parseInt((card as HTMLElement).dataset.pageNum || "1", 10);
      
      // Remove old badges on this card
      card.querySelectorAll(".pdf-marker-badge").forEach(b => b.remove());

      const pageMarkers = markers.filter(m => m.pdfAnchor && m.pdfAnchor.page === pageNum);
      for (const m of pageMarkers) {
        if (!m.pdfAnchor) continue;
        const badge = document.createElement("div");
        badge.className = "pdf-marker-badge";
        badge.dataset.markerId = m.id.toString();
        badge.style.left = `${(m.pdfAnchor.xPct * 100).toFixed(2)}%`;
        badge.style.top = `${(m.pdfAnchor.yPct * 100).toFixed(2)}%`;
        badge.style.backgroundColor = m.color || "#ff9500";
        badge.title = `Marker: ${m.name} (${formatTime(m.time)}) • Click to jump • Drag to move (drag off to unpin)`;

        badge.innerHTML = `
          <span class="pdf-marker-dot"></span>
          <span class="pdf-marker-title">${m.name}</span>
          <button class="pdf-marker-unpin" title="Unpin from score">×</button>
        `;

        // Click to seek
        badge.addEventListener("click", async (e) => {
          e.stopPropagation();
          currentTime = m.time;
          progress = duration > 0 ? currentTime / duration : 0;
          await invoke("seek", { seconds: m.time });
          updateVisiblePeaks();
          drawMainWaveform();
          drawOverviewWaveform();
        });

        // Unpin button
        const unpinBtn = badge.querySelector(".pdf-marker-unpin");
        if (unpinBtn) {
          unpinBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            removeMarkerPdfAnchor(m.id);
          });
        }

        // Mouse pointer drag to reposition or unpin
        badge.addEventListener("mousedown", (e) => {
          if ((e.target as HTMLElement).classList.contains("pdf-marker-unpin")) return;
          e.stopPropagation();
          startMarkerPointerDrag(e, m, true);
        });

        card.appendChild(badge);
      }
    });
  }

  // Reactive trigger when switching to PDF tab or when pdfChartPath changes
  $: if (activeCenterTab === "pdf" && pdfChartPath) {
    tick().then(() => {
      renderPdfPages();
    });
  }

  async function loadTrackProfile(trackPath: string, isMainSong = true) {
    const store = getProfilesStore();
    const profile = store[trackPath];
    if (profile) {
      dbVolume = typeof profile.dbVolume === "number" ? profile.dbVolume : 0.0;
      speed = typeof profile.speed === "number" ? profile.speed : 1.0;
      pitch = typeof profile.pitch === "number" ? profile.pitch : 0;
      pitchCents = typeof profile.pitchCents === "number" ? profile.pitchCents : 0;
      eqBass = typeof profile.eqBass === "number" ? profile.eqBass : 0;
      eqMid = typeof profile.eqMid === "number" ? profile.eqMid : 0;
      eqTreble = typeof profile.eqTreble === "number" ? profile.eqTreble : 0;
      isEqBypassed = !!profile.isEqBypassed;
      if (Array.isArray(profile.eqNodes) && profile.eqNodes.length > 0) {
        eqNodes = [...profile.eqNodes];
      } else {
        eqNodes = [
          { id: "node-1", name: "Low Shelf", filterType: "LowShelf", freq: 100, gainDb: eqBass, q: 0.707, enabled: true, color: "#3b99fc" },
          { id: "node-2", name: "Mid Bell", filterType: "Peaking", freq: 1000, gainDb: eqMid, q: 1.0, enabled: true, color: "#30d158" },
          { id: "node-3", name: "High Shelf", filterType: "HighShelf", freq: 8000, gainDb: eqTreble, q: 0.707, enabled: true, color: "#ff9500" },
        ];
      }

      compressorThreshold = typeof profile.compressorThreshold === "number" ? profile.compressorThreshold : 0.0;
      compressorRatio = typeof profile.compressorRatio === "number" ? profile.compressorRatio : 1.0;
      compressorMakeup = typeof profile.compressorMakeup === "number" ? profile.compressorMakeup : 0.0;
      isCompressorBypassed = !!profile.isCompressorBypassed;

      if (profile.compStage1) {
        compStage1 = { ...profile.compStage1 };
      } else {
        compStage1 = {
          enabled: true,
          compType: "Vintage",
          thresholdDb: compressorThreshold,
          ratio: compressorRatio,
          kneeDb: 3.0,
          attackMs: 30.0,
          releaseMs: 300.0,
          makeupDb: compressorMakeup,
          limiter: false
        };
      }

      if (profile.compStage2) {
        compStage2 = { ...profile.compStage2 };
      } else {
        compStage2 = {
          enabled: false,
          compType: "Opto",
          thresholdDb: -12.0,
          ratio: 2.0,
          kneeDb: 4.0,
          attackMs: 50.0,
          releaseMs: 500.0,
          makeupDb: 0.0,
          limiter: false
        };
      }

      compRouting = profile.compRouting || "Series";
      compParallelBlend = typeof profile.compParallelBlend === "number" ? profile.compParallelBlend : 0.5;

      markers = Array.isArray(profile.markers) ? [...profile.markers] : [];
      nextMarkerId = typeof profile.nextMarkerId === "number" ? profile.nextMarkerId : (markers.length + 1);
      regions = Array.isArray(profile.regions) ? [...profile.regions] : [];
      nextRegionId = typeof profile.nextRegionId === "number" ? profile.nextRegionId : (regions.length + 1);

      if (isMainSong) {
        pdfChartPath = profile.pdfChartPath || "";
        pdfChartName = profile.pdfChartName || (pdfChartPath ? (pdfChartPath.split("/").pop() || "") : "");
        associatedVersions = Array.isArray(profile.associatedVersions) ? profile.associatedVersions : [];
        
        // Populate associatedFiles
        if (Array.isArray(profile.associatedFiles)) {
          associatedFiles = [...profile.associatedFiles];
        } else {
          associatedFiles = [];
          if (pdfChartPath) {
            associatedFiles.push({
              id: "pdf-main",
              name: pdfChartName || "Sheet Music.pdf",
              path: pdfChartPath,
              fileType: "pdf"
            });
          }
          for (const ver of associatedVersions) {
            associatedFiles.push({
              id: "track-" + ver.path,
              name: ver.name,
              path: ver.path,
              fileType: "audio"
            });
          }
        }

        songNotes = profile.notes || "";
        songLyrics = profile.lyrics || "";
        notesViewMode = profile.notesViewMode || "edit";
        lyricsViewMode = profile.lyricsViewMode || "edit";

        if (profile.lastCenterTab) {
          activeCenterTab = profile.lastCenterTab;
        } else {
          activeCenterTab = "notes";
        }
        
        if (profile.alternateTrackPath && profile.alternateTrackPath !== trackPath) {
          loadAudioPath(profile.alternateTrackPath, "alternate", false);
        } else {
          alternateTrack = null;
        }
      }
    } else {
      // Default clean settings for newly loaded song or fresh alternate track
      dbVolume = 0.0;
      speed = 1.0;
      pitch = 0;
      pitchCents = 0;
      eqBass = 0.0;
      eqMid = 0.0;
      eqTreble = 0.0;
      isEqBypassed = false;
      eqNodes = [
        { id: "node-1", name: "Low Shelf", filterType: "LowShelf", freq: 100, gainDb: 0, q: 0.707, enabled: true, color: "#3b99fc" },
        { id: "node-2", name: "Mid Bell", filterType: "Peaking", freq: 1000, gainDb: 0, q: 1.0, enabled: true, color: "#30d158" },
        { id: "node-3", name: "High Shelf", filterType: "HighShelf", freq: 8000, gainDb: 0, q: 0.707, enabled: true, color: "#ff9500" },
      ];

      compressorThreshold = 0.0;
      compressorRatio = 1.0;
      compressorMakeup = 0.0;
      isCompressorBypassed = false;
      compStage1 = {
        enabled: true,
        compType: "Vintage",
        thresholdDb: 0.0,
        ratio: 1.0,
        kneeDb: 3.0,
        attackMs: 30.0,
        releaseMs: 300.0,
        makeupDb: 0.0,
        limiter: false
      };
      compStage2 = {
        enabled: false,
        compType: "Opto",
        thresholdDb: -12.0,
        ratio: 2.0,
        kneeDb: 4.0,
        attackMs: 50.0,
        releaseMs: 500.0,
        makeupDb: 0.0,
        limiter: false
      };
      compRouting = "Series";
      compParallelBlend = 0.5;

      markers = [];
      nextMarkerId = 1;
      regions = [];
      nextRegionId = 1;

      if (isMainSong) {
        pdfChartPath = "";
        pdfChartName = "";
        associatedVersions = [];
        associatedFiles = [];
        songNotes = "";
        songLyrics = "";
        notesViewMode = "edit";
        lyricsViewMode = "edit";
        alternateTrack = null;
        activeCenterTab = "notes";
      }
    }

    // Apply restored volume, speed, pitch, EQ, compressor, and regions directly to audio engine
    const linearVol = dbVolume <= -59.5 ? 0 : Math.pow(10, dbVolume / 20);
    await invoke("set_volume", { volume: linearVol });
    await invoke("set_speed", { speed });
    const totalSemitones = pitch + (pitchCents / 100.0);
    await invoke("set_pitch", { pitch: totalSemitones });
    await updateEqEngine();
    await updateCompressorEngine();
    await syncRegionsToEngine();
  }

  // Canvas elements & Offscreen static render caches
  let mainCanvas: HTMLCanvasElement;
  let overviewCanvas: HTMLCanvasElement;
  let centerContentElement: HTMLDivElement;

  let mainStaticCanvas: HTMLCanvasElement | null = null;
  let isMainStaticDirty = true;
  let overviewBaseCanvas: HTMLCanvasElement | null = null;
  let isOverviewBaseDirty = true;

  function invalidateWaveformCaches() {
    isMainStaticDirty = true;
    isOverviewBaseDirty = true;
    isWaveformDirty = true;
  }
  
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

    const savedInvert = localStorage.getItem("th_pdf_inverted");
    if (savedInvert === "1") isPdfInverted = true;

    const savedCenterTab = localStorage.getItem("th_last_center_tab") as "notes" | "lyrics" | "metadata" | "pdf" | null;
    if (savedCenterTab) activeCenterTab = savedCenterTab;

    const savedVersions = localStorage.getItem("th_associated_versions");
    if (savedVersions) associatedVersions = JSON.parse(savedVersions);

    // Restore last loaded tracks and active state
    (async () => {
      const lastMainPath = localStorage.getItem("th_last_main_track_path");
      const lastAltPath = localStorage.getItem("th_last_alt_track_path");
      const lastActiveMode = (localStorage.getItem("th_last_active_track_mode") as "main" | "alternate" | null) || "main";

      try {
        if (lastActiveMode === "alternate" && lastAltPath) {
          if (lastMainPath) {
            await loadAudioPath(lastMainPath, "main", false);
          }
          await loadAudioPath(lastAltPath, "alternate", true);
        } else {
          if (lastAltPath) {
            await loadAudioPath(lastAltPath, "alternate", false);
          }
          if (lastMainPath) {
            await loadAudioPath(lastMainPath, "main", true);
          }
        }
      } catch (err) {
        console.error("Failed to restore last loaded tracks:", err);
      }
    })();

    statusInterval = setInterval(async () => {
      try {
        const status: any = await invoke("get_playback_status");
        const wasPlaying = isPlaying;
        isPlaying = status.is_playing;
        if (!isPanning && !isDraggingOverview) {
          currentTime = status.current_time;
          duration = status.duration_seconds;
          progress = status.progress;
          
          if (isPlaying || wasPlaying || isWaveformDirty) {
            isWaveformDirty = false;
            if (zoom > 1.001) {
              updateVisiblePeaks();
            }
            drawMainWaveform();
            drawOverviewWaveform();
          }

          // Broadcast state to connected WebSocket clients (Stream Deck, Web remotes, Bitfocus Companion)
          if (filePath) {
            let activeMarkerName = "";
            for (const m of markers) {
              if (m.time <= currentTime + 0.15) {
                if (!activeMarkerName || m.time > (markers.find(x => x.name === activeMarkerName)?.time ?? 0)) {
                  activeMarkerName = m.name;
                }
              }
            }

            const statePayload = JSON.stringify({
              type: "state",
              isPlaying,
              currentTime,
              duration,
              formattedTime: formatTime(currentTime),
              formattedRemaining: formatTime(Math.max(0, duration - currentTime)),
              trackName: fileName,
              filePath,
              playlistIndex: selectedPlaylistIndex >= 0 ? selectedPlaylistIndex + 1 : 0,
              playlistTotal: playlistItems.length,
              currentMarker: activeMarkerName,
              pitchSemitones: pitch + (pitchCents / 100.0),
              volumeDb: volume,
              speed,
              isLooping: regions.some(r => r.isLoop && currentTime >= r.startTime && currentTime <= r.endTime)
            });
            invoke("broadcast_remote_state", { stateJson: statePayload }).catch(() => {});
          }

          // Auto-scroll PDF to active anchored marker (12px below top)
          if (activeCenterTab === "pdf" && pdfContainer && isPlaying) {
            let currentMarker: Marker | null = null;
            for (const m of markers) {
              if (m.time <= currentTime + 0.15) {
                if (!currentMarker || m.time > currentMarker.time) {
                  currentMarker = m;
                }
              }
            }

            // Reset scrolled marker if playhead is before the marker
            if (!currentMarker || currentTime < currentMarker.time - 0.5) {
              lastScrolledMarkerId = null;
            }

            if (currentMarker && currentMarker.id !== lastScrolledMarkerId) {
              let anchor = currentMarker.pdfAnchor;
              if (!anchor && activeTrackMode === "alternate" && mainTrack) {
                const mainMarkers = getProfilesStore()[mainTrack.path]?.markers || [];
                const match = mainMarkers.find(mm => mm.name.trim().toLowerCase() === currentMarker!.name.trim().toLowerCase());
                if (match && match.pdfAnchor) {
                  anchor = match.pdfAnchor;
                }
              }

              if (anchor) {
                lastScrolledMarkerId = currentMarker.id;
                const markerPage = anchor.page;
                const card = pdfContainer.querySelector(`.pdf-page-card[data-page-num="${markerPage}"]`) as HTMLElement;
                if (card) {
                  const containerRect = pdfContainer.getBoundingClientRect();
                  const cardRect = card.getBoundingClientRect();
                  const cardTopRelativeToContainer = (cardRect.top - containerRect.top) + pdfContainer.scrollTop;
                  const markerYWithinCard = card.clientHeight * anchor.yPct;
                  const targetScrollTop = Math.max(0, cardTopRelativeToContainer + markerYWithinCard - 12);
                  pdfContainer.scrollTo({
                    top: targetScrollTop,
                    behavior: "smooth"
                  });
                }
              }
            }
          }
        }
      } catch (err) {
        console.error("Failed to query playback status", err);
      }
    }, 50);

    // Resize observer for canvases
    if (centerContentElement) {
      resizeObserver = new ResizeObserver(() => {
        invalidateWaveformCaches();
        drawMainWaveform();
        drawOverviewWaveform();
      });
      resizeObserver.observe(centerContentElement);
    }

    // Listen to native window file drops (Tauri drag & drop)
    const unlistenDragDrop = listen("tauri://drag-drop", (event: any) => {
      const paths = event.payload.paths;
      if (paths && paths.length > 0) {
        const playlistFile = paths.find((p: string) => {
          const lower = p.toLowerCase();
          return lower.endsWith(".thset") || lower.endsWith(".m3u8") || lower.endsWith(".m3u") || (lower.endsWith(".json") && !lower.endsWith("profile.json"));
        });
        if (playlistFile) {
          loadPlaylistFromFile(playlistFile);
          return;
        }

        const audioPath = paths.find((p: string) => {
          const lower = p.toLowerCase();
          return lower.endsWith(".wav") || lower.endsWith(".mp3") || lower.endsWith(".flac") || lower.endsWith(".m4a") || lower.endsWith(".aiff") || lower.endsWith(".ogg");
        });
        if (audioPath) {
          loadAudioPath(audioPath, "main");
        }
      }
    });

    // Close context menu & color palette on window click
    const closeMenu = () => { 
      showContextMenu = false; 
      colorPaletteMarker = null;
    };
    window.addEventListener("click", closeMenu);

    // Listen to native macOS Menu Bar events
    const unlistenMenu = listen("menu-action", (event: any) => {
      const action = event.payload;
      if (action === "open_file") pickMainTrack();
      else if (action === "open_alternate") pickAlternateTrack();
      else if (action === "open_playlist") loadPlaylistFromFile();
      else if (action === "save_playlist") savePlaylistToFile();
      else if (action === "export_audio") openExportModal();
      else if (action === "add_marker") addMarker();
      else if (action === "create_region") createRegionFromSelectionOrMarkers();
      else if (action === "toggle_loop") handleLoopHotkey();
      else if (action === "toggle_cut") handleCutHotkey();
      else if (action === "play_pause") handlePlayPause();
      else if (action === "stop") handleStop();
      else if (action === "prev_marker") jumpToPrevMarker();
      else if (action === "next_marker") jumpToNextMarker();
      else if (action === "tab_notes") switchCenterTab("notes");
      else if (action === "tab_lyrics") switchCenterTab("lyrics");
      else if (action === "tab_metadata") switchCenterTab("metadata");
      else if (action === "tab_files") switchCenterTab("files");
      else if (action === "open_remotes") openRemoteSettingsModal();
    });

    // Listen to Remote Show Control commands (WebSocket / OSC)
    const unlistenRemote = listen("remote-control-action", (event: any) => {
      handleRemoteControlAction(event.payload);
    });

    // Listen to Hardware MIDI events
    const unlistenMidi = listen("midi-event", (event: any) => {
      handleMidiEvent(event.payload);
    });

    // Global keyboard shortcuts: Space = Play/Pause/Load, Up/Down = Playlist/Browser Nav, Left/Right = Marker Jump, M = Add Marker, Enter = Stop & Return to 0, Cmd+Shift+E = Export Audio, Cmd+Shift+R = Remote Settings
    const handleKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }

      if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.code === "KeyR" || e.key === "r" || e.key === "R")) {
        e.preventDefault();
        openRemoteSettingsModal();
        return;
      }

      if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.code === "KeyE" || e.key === "e" || e.key === "E")) {
        e.preventDefault();
        openExportModal();
        return;
      }

      if (e.code === "Space") {
        e.preventDefault();
        
        // Option 2: If actively navigating playlist and a different track is highlighted, load and immediately play!
        if (activeTab === "playlist" && selectedPlaylistIndex >= 0 && selectedPlaylistIndex < playlistItems.length) {
          const highlighted = playlistItems[selectedPlaylistIndex];
          if (highlighted && highlighted.path !== filePath) {
            loadAudioPath(highlighted.path, "main", true).then(async () => {
              await invoke("play");
              isPlaying = true;
            });
            return;
          }
        }

        // Standard Play/Pause toggle
        handlePlayPause();
      } else if (e.code === "ArrowUp") {
        e.preventDefault();
        if (activeTab === "playlist" && playlistItems.length > 0) {
          let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.max(0, currentIdx - 1);
          selectedPlaylistIndex = currentIdx;
          scrollSelectedPlaylistItemIntoView();
        } else if (activeTab === "browser" && filteredEntries.length > 0) {
          let currentIdx = filteredEntries.findIndex(e => selectedFilePaths.has(e.path));
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.max(0, currentIdx - 1);
          const targetEntry = filteredEntries[currentIdx];
          if (targetEntry) {
            selectedFilePaths.clear();
            selectedFilePaths.add(targetEntry.path);
            selectedFilePaths = selectedFilePaths;
            lastSelectedEntry = targetEntry;
            scrollSelectedBrowserItemIntoView();
          }
        }
      } else if (e.code === "ArrowDown") {
        e.preventDefault();
        if (activeTab === "playlist" && playlistItems.length > 0) {
          let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.min(playlistItems.length - 1, currentIdx + 1);
          selectedPlaylistIndex = currentIdx;
          scrollSelectedPlaylistItemIntoView();
        } else if (activeTab === "browser" && filteredEntries.length > 0) {
          let currentIdx = filteredEntries.findIndex(e => selectedFilePaths.has(e.path));
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.min(filteredEntries.length - 1, currentIdx + 1);
          const targetEntry = filteredEntries[currentIdx];
          if (targetEntry) {
            selectedFilePaths.clear();
            selectedFilePaths.add(targetEntry.path);
            selectedFilePaths = selectedFilePaths;
            lastSelectedEntry = targetEntry;
            scrollSelectedBrowserItemIntoView();
          }
        }
      } else if (e.code === "KeyM" || e.key === "m" || e.key === "M") {
        if (!e.metaKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          addMarker();
        }
      } else if (e.code === "KeyR" || e.key === "r" || e.key === "R") {
        if (!e.metaKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          createRegionFromSelectionOrMarkers();
        }
      } else if (e.code === "KeyX" || e.key === "x" || e.key === "X") {
        if (!e.metaKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          handleCutHotkey();
        }
      } else if (e.code === "KeyL" || e.key === "l" || e.key === "L") {
        if (!e.metaKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          handleLoopHotkey();
        }
      } else if (e.code === "ArrowLeft") {
        e.preventDefault();
        jumpToPrevMarker();
      } else if (e.code === "ArrowRight") {
        e.preventDefault();
        jumpToNextMarker();
      } else if (e.code === "Enter" || e.code === "NumpadEnter") {
        e.preventDefault();
        if (activeTab === "browser" && lastSelectedEntry) {
          if (lastSelectedEntry.is_dir) {
            loadBrowser(lastSelectedEntry.path);
          } else {
            loadAudioPath(lastSelectedEntry.path, "main", true).then(async () => {
              await invoke("play");
              isPlaying = true;
            });
          }
        } else {
          handleStop();
        }
      } else if (e.code === "Escape") {
        e.preventDefault();
        showContextMenu = false;
        colorPaletteMarker = null;
        cancelRenameMarker();
        handleStop();
      }
    };
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      clearInterval(statusInterval);
      if (resizeObserver) resizeObserver.disconnect();
      unlistenDragDrop.then(fn => fn());
      unlistenMenu.then(fn => fn());
      unlistenRemote.then(fn => fn());
      unlistenMidi.then(fn => fn());
      window.removeEventListener("click", closeMenu);
      window.removeEventListener("keydown", handleKeyDown);
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

  // Background Preloading Engine for Instantaneous Live Set Switching
  const preloadedTrackMetadata = new Map<string, any>();
  const inFlightPreloads = new Set<string>();
  let isPreloading = false;

  async function preloadAdjacentTracks() {
    if (isPreloading) return;
    isPreloading = true;

    try {
      const candidates: string[] = [];

      // 1. Next & Previous track in active playlist
      if (playlistItems.length > 0) {
        const pIdx = playlistItems.findIndex(p => p.path === filePath);
        if (pIdx !== -1) {
          if (pIdx + 1 < playlistItems.length) {
            candidates.push(playlistItems[pIdx + 1].path);
          }
          if (pIdx > 0) {
            candidates.push(playlistItems[pIdx - 1].path);
          }
        } else {
          candidates.push(playlistItems[0].path);
        }
      }

      // 2. Next & Previous track in current folder browser
      const audioEntries = filteredEntries.filter(e => !e.is_dir);
      if (audioEntries.length > 0) {
        const fIdx = audioEntries.findIndex(e => e.path === filePath);
        if (fIdx !== -1) {
          if (fIdx + 1 < audioEntries.length) {
            candidates.push(audioEntries[fIdx + 1].path);
          }
          if (fIdx > 0) {
            candidates.push(audioEntries[fIdx - 1].path);
          }
        }
      }

      // 3. Alternate track and associated versions
      if (alternateTrack && alternateTrack.path && alternateTrack.path !== filePath) {
        candidates.push(alternateTrack.path);
      }
      for (const assoc of associatedVersions) {
        if (assoc.path && assoc.path !== filePath) {
          candidates.push(assoc.path);
        }
      }

      // Preload candidates asynchronously without duplicate in-flight requests
      for (const path of candidates) {
        if (!path || preloadedTrackMetadata.has(path) || inFlightPreloads.has(path)) continue;
        
        inFlightPreloads.add(path);
        // Background decode audio and compute peaks into memory cache
        invoke("preload_track", { path })
          .then((meta: any) => {
            if (meta) {
              preloadedTrackMetadata.set(path, meta);
            }
          })
          .catch(() => {})
          .finally(() => {
            inFlightPreloads.delete(path);
          });

        // Background preload audio tags
        invoke("read_audio_metadata", { path }).catch(() => {});
      }
    } finally {
      isPreloading = false;
    }
  }

  // Load a file into main or alternate track slots
  async function loadAudioPath(path: string, target: "main" | "alternate", switchActive = (target === "main")) {
    try {
      const wasPlaying = isPlaying;

      // Save current track profile before switching
      if (switchActive && filePath) {
        saveCurrentTrackProfile(filePath);
      }

      const metadata: any = switchActive
        ? await invoke("load_track", { path })
        : await invoke("preload_track", { path });

      const track: Track = {
        name: path.split("/").pop() || path,
        path: path,
        duration: metadata.duration_seconds,
        sampleRate: metadata.sample_rate,
        channels: metadata.channels,
        overviewPeaks: metadata.overview_peaks || [],
        pyramidPeaks: metadata.pyramid_peaks || []
      };

      if (target === "main") {
        mainTrack = track;
        localStorage.setItem("th_last_main_track_path", path);
      } else {
        alternateTrack = track;
        localStorage.setItem("th_last_alt_track_path", path);
        if (mainTrack) {
          saveCurrentTrackProfile(mainTrack.path);
        }
      }

      if (switchActive) {
        filePath = path;
        fileName = track.name;
        duration = track.duration;
        sampleRate = track.sampleRate;
        channels = track.channels;
        currentTime = 0;
        progress = 0;
        activeTrackMode = target;
        localStorage.setItem("th_last_active_track_mode", target);

        // Clear local sample cache
        localSampleCache = { startFrame: -1, endFrame: -1, samples: [] };

        // Default zoom: 15 seconds rehearsal chunk view
        if (duration > 0) {
          const targetZoom = Math.max(1.0, duration / 15.0);
          setZoom(targetZoom);
        } else {
          setZoom(1.0);
        }

        // Restore saved Track Profile
        if (target === "main") {
          await loadTrackProfile(path, true);
        } else {
          await loadTrackProfile(path, false);
        }

        // Seamless live playback rollover: continue playing if was playing
        if (wasPlaying) {
          await invoke("play");
          isPlaying = true;
        }

        // Read audio tags
        invoke("read_audio_metadata", { path }).then((res: any) => {
          audioTags = res || {};
          if (audioTags.title) {
            fileName = audioTags.title;
          }
        }).catch(() => {
          audioTags = {};
        });

        setTimeout(() => {
          updateVisiblePeaks();
          preloadAdjacentTracks();
        }, 30);
      }
    } catch (err) {
      alert("Failed to load file: " + err);
    }
  }

  // Toggle active track between Main and Alternate (with Smart Relative Landmark Sync)
  async function toggleActiveTrack(target: "main" | "alternate") {
    const targetTrack = target === "main" ? mainTrack : alternateTrack;
    const sourceTrack = activeTrackMode === "main" ? mainTrack : alternateTrack;
    if (!targetTrack) {
      if (target === "alternate") {
        await pickAlternateTrack();
      } else {
        await pickMainTrack();
      }
      return;
    }

    const wasPlaying = isPlaying;
    const sourceTime = currentTime;

    // 1. Calculate smart landmark sync time if both tracks share markers
    let targetTime = sourceTime;
    if (sourceTrack && targetTrack && sourceTrack.path !== targetTrack.path) {
      const sourceMarkers = markers || [];
      const store = getProfilesStore();
      const targetProfile = store[targetTrack.path];
      const targetMarkers: Marker[] = targetProfile && Array.isArray(targetProfile.markers) ? targetProfile.markers : [];

      if (sourceMarkers.length > 0 && targetMarkers.length > 0) {
        // Find preceding shared landmark
        let bestPrecedingSource: Marker | null = null;
        let bestPrecedingTarget: Marker | null = null;

        for (const sm of sourceMarkers) {
          if (sm.time <= sourceTime) {
            const tm = targetMarkers.find(m => m.name.trim().toLowerCase() === sm.name.trim().toLowerCase());
            if (tm) {
              if (!bestPrecedingSource || sm.time > bestPrecedingSource.time) {
                bestPrecedingSource = sm;
                bestPrecedingTarget = tm;
              }
            }
          }
        }

        if (bestPrecedingSource && bestPrecedingTarget) {
          const delta = sourceTime - bestPrecedingSource.time;
          targetTime = bestPrecedingTarget.time + delta;
        } else {
          // Check for following shared landmark
          let bestFollowingSource: Marker | null = null;
          let bestFollowingTarget: Marker | null = null;
          for (const sm of sourceMarkers) {
            if (sm.time > sourceTime) {
              const tm = targetMarkers.find(m => m.name.trim().toLowerCase() === sm.name.trim().toLowerCase());
              if (tm) {
                if (!bestFollowingSource || sm.time < bestFollowingSource.time) {
                  bestFollowingSource = sm;
                  bestFollowingTarget = tm;
                }
              }
            }
          }

          if (bestFollowingSource && bestFollowingTarget) {
            const delta = sourceTime - bestFollowingSource.time;
            targetTime = Math.max(0, bestFollowingTarget.time + delta);
          }
        }
      }
    }

    targetTime = Math.max(0, Math.min(targetTrack.duration, targetTime));

    // Save outgoing profile
    if (sourceTrack) {
      saveCurrentTrackProfile(sourceTrack.path);
    }

    activeTrackMode = target;
    filePath = targetTrack.path;
    fileName = targetTrack.name;
    duration = targetTrack.duration;
    sampleRate = targetTrack.sampleRate;
    channels = targetTrack.channels;
    currentTime = targetTime;
    progress = duration > 0 ? currentTime / duration : 0;
    localStorage.setItem("th_last_active_track_mode", target);

    // Clear local sample cache so waveform redraws fresh samples
    localSampleCache = { startFrame: -1, endFrame: -1, samples: [] };

    // Load track into backend engine
    await invoke("load_track", { path: targetTrack.path });

    // Restore target track's own profile (Gain, Pitch, Speed, EQ, Compression, Markers)
    await loadTrackProfile(targetTrack.path, false);

    // Restore playhead position
    if (targetTime > 0 && targetTime < targetTrack.duration) {
      await invoke("seek", { seconds: targetTime });
    }

    if (wasPlaying) {
      await invoke("play");
    }

    // Read audio tags for active track
    invoke("read_audio_metadata", { path: targetTrack.path }).then((res: any) => {
      audioTags = res || {};
      if (audioTags.title) {
        fileName = audioTags.title;
      }
    }).catch(() => {
      audioTags = {};
    });

    lastScrolledMarkerId = null;
    invalidateWaveformCaches();
    await updateVisiblePeaks();
    drawMainWaveform();
    drawOverviewWaveform();
    if (activeCenterTab === "pdf") {
      renderPdfMarkerBadges();
    }
  }

  function handleWaveformMarkerDrop(e: DragEvent) {
    e.preventDefault();
    if (!mainCanvas || duration === 0) return;
    const markerIdStr = e.dataTransfer?.getData("text/trackhelm-marker-id");
    if (!markerIdStr) return;
    const markerId = parseInt(markerIdStr, 10);

    const rect = mainCanvas.getBoundingClientRect();
    const dropX = e.clientX - rect.left;
    const clickPct = Math.max(0, Math.min(1.0, dropX / rect.width));
    const windowWidth = 1.0 / zoom;
    const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
    const dropTime = (startProgress + clickPct * windowWidth) * duration;

    let existing = markers.find(m => m.id === markerId);
    if (existing) {
      existing.time = dropTime;
    } else {
      const allProfiles = getProfilesStore();
      let foundName = `Marker ${nextMarkerId}`;
      let foundColor = MARKER_COLORS[(markers.length) % MARKER_COLORS.length];
      for (const p of Object.values(allProfiles)) {
        const sm = p.markers?.find(m => m.id === markerId);
        if (sm) {
          foundName = sm.name;
          foundColor = sm.color || foundColor;
          break;
        }
      }
      markers = [...markers, { id: nextMarkerId++, name: foundName, time: dropTime, color: foundColor }];
    }

    markers.sort((a, b) => a.time - b.time);
    markers = markers;
    saveCurrentTrackProfile(filePath);
    drawMainWaveform();
    drawOverviewWaveform();
    if (activeCenterTab === "pdf") {
      renderPdfMarkerBadges();
    }
  }

  async function pickAlternateTrack() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Audio Files", extensions: ["wav", "mp3", "flac", "m4a", "aiff", "ogg"] }]
      });
      if (selected && typeof selected === "string") {
        await loadAudioPath(selected, "alternate", true);
      }
    } catch (err) {
      alert("Failed to select alternate track: " + err);
    }
  }

  async function pickMainTrack() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Audio Files", extensions: ["wav", "mp3", "flac", "m4a", "aiff", "ogg"] }]
      });
      if (selected && typeof selected === "string") {
        await loadAudioPath(selected, "main", true);
      }
    } catch (err) {
      alert("Failed to select main track: " + err);
    }
  }

  function performTypeToJump() {
    if (!typeToJumpBuffer) return;
    
    if (activeTab === "playlist" && playlistItems.length > 0) {
      const matchIdx = playlistItems.findIndex(entry => 
        entry.name.toLowerCase().startsWith(typeToJumpBuffer.toLowerCase())
      );
      if (matchIdx !== -1) {
        selectedPlaylistIndex = matchIdx;
        scrollSelectedPlaylistItemIntoView();
      }
      return;
    }

    // Otherwise File Browser
    const match = filteredEntries.find(entry => 
      entry.name.toLowerCase().startsWith(typeToJumpBuffer.toLowerCase())
    );

    if (match) {
      selectedFilePaths.clear();
      selectedFilePaths.add(match.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: match.name, path: match.path };
      scrollSelectedBrowserItemIntoView();
    }
  }

  function scrollSelectedPlaylistItemIntoView() {
    setTimeout(() => {
      const el = document.querySelector(".playlist-item-sidebar.highlighted") || document.querySelector(".playlist-item-sidebar.active");
      if (el) {
        el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
    }, 20);
  }

  function scrollSelectedBrowserItemIntoView() {
    setTimeout(() => {
      const el = document.querySelector(".browser-item.active");
      if (el) {
        el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
    }, 20);
  }

  // Handle Multi-select File Clicks & Background Prefetching
  function handleFileClick(e: MouseEvent, entry: any) {
    if (entry.is_dir) return;

    // Trigger instant background pre-decoding in Rust
    invoke("preload_track", { path: entry.path }).catch(() => {});

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

    contextMenuType = "browser";
    contextMenuTargetFile = { name: entry.name, path: entry.path };
    contextMenuX = e.clientX;
    contextMenuY = e.clientY;
    showContextMenu = true;
  }

  function handleWaveformContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    contextMenuType = "waveform";
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

  async function savePlaylistToFile() {
    if (playlistItems.length === 0) {
      alert("Playlist is empty. Add audio tracks before saving.");
      return;
    }
    try {
      const selected = await save({
        defaultPath: "Setlist.thset",
        filters: [
          { name: "TrackHelm Set (*.thset)", extensions: ["thset"] },
          { name: "M3U8 Playlist (*.m3u8)", extensions: ["m3u8"] },
          { name: "M3U Playlist (*.m3u)", extensions: ["m3u"] },
          { name: "JSON Playlist (*.json)", extensions: ["json"] }
        ]
      });
      if (selected && typeof selected === "string") {
        const ext = selected.split(".").pop()?.toLowerCase() || "thset";
        const format = (ext === "m3u8" || ext === "m3u") ? "m3u8" : (ext === "json" ? "json" : "thset");
        await invoke("save_playlist_file", {
          path: selected,
          format,
          items: playlistItems.map(p => ({
            name: p.name,
            path: p.path,
            duration: p.duration
          }))
        });
      }
    } catch (err) {
      console.error("Failed to save playlist:", err);
      alert("Failed to save playlist: " + err);
    }
  }

  async function loadPlaylistFromFile(directPath?: string) {
    try {
      let targetPath = directPath;
      if (!targetPath) {
        const selected = await open({
          multiple: false,
          filters: [
            { name: "Playlists & Sets (*.thset, *.m3u8, *.m3u, *.json)", extensions: ["thset", "m3u8", "m3u", "json"] }
          ]
        });
        if (selected && typeof selected === "string") {
          targetPath = selected;
        }
      }
      if (!targetPath) return;

      const loadedItems: PlaylistItem[] = await invoke("load_playlist_file", { path: targetPath });
      if (!loadedItems || loadedItems.length === 0) {
        alert("No audio tracks found in playlist file.");
        return;
      }

      if (playlistItems.length > 0) {
        const shouldAppend = confirm(`Found ${loadedItems.length} track(s).\n\nClick OK to REPLACE current list, or Cancel to APPEND to it.`);
        if (shouldAppend) {
          playlistItems = loadedItems;
        } else {
          const existingPaths = new Set(playlistItems.map(p => p.path));
          const newEntries = loadedItems.filter(p => !existingPaths.has(p.path));
          playlistItems = [...playlistItems, ...newEntries];
        }
      } else {
        playlistItems = loadedItems;
      }
      localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
      activeTab = "playlist";
    } catch (err) {
      console.error("Failed to load playlist:", err);
      alert("Failed to load playlist: " + err);
    }
  }

  // Audio Export Modal Functions
  function openExportModal() {
    if (!filePath || duration === 0) {
      alert("Please load an audio track first before exporting.");
      return;
    }
    if (timeSelection && (timeSelection.end - timeSelection.start) > 0.1) {
      exportRange = "selection";
    } else {
      exportRange = "full";
    }
    if (regions.length > 0) {
      exportSelectedRegionId = regions[0].id;
    }
    exportStatusMessage = "";
    exportErrorMessage = "";
    showExportModal = true;
  }

  async function executeAudioExport() {
    if (!filePath) return;
    try {
      isExporting = true;
      exportStatusMessage = "Preparing audio export...";
      exportErrorMessage = "";

      let rangeStart: number | undefined = undefined;
      let rangeEnd: number | undefined = undefined;

      if (exportRange === "selection" && timeSelection) {
        rangeStart = timeSelection.start;
        rangeEnd = timeSelection.end;
      } else if (exportRange === "region" && exportSelectedRegionId) {
        const targetReg = regions.find(r => r.id === exportSelectedRegionId);
        if (targetReg) {
          rangeStart = targetReg.startTime;
          rangeEnd = targetReg.endTime;
        }
      }

      // Generate suggested file name
      const baseName = fileName.replace(/\.[^/.]+$/, "");
      let suffix = "";
      if (exportBakeSpeed && Math.abs(speed - 1.0) > 0.01) {
        suffix += `_${speed.toFixed(2)}x`;
      }
      if (exportBakePitch && (pitch !== 0 || pitchCents !== 0)) {
        const totalSemi = pitch + (pitchCents / 100.0);
        suffix += `_${totalSemi > 0 ? "+" : ""}${totalSemi.toFixed(1)}st`;
      }
      const ext = exportBitDepth === "float32" ? "float32.wav" : (exportBitDepth === "int16" ? "16bit.wav" : "24bit.wav");
      const defaultFileName = `${baseName}${suffix}_export.${ext}`;

      const selectedOut = await save({
        defaultPath: defaultFileName,
        filters: [{ name: "WAV Audio (*.wav)", extensions: ["wav"] }]
      });

      if (!selectedOut || typeof selectedOut !== "string") {
        isExporting = false;
        return;
      }

      exportStatusMessage = "Processing DSP and rendering output file...";

      const totalSemitones = pitch + (pitchCents / 100.0);

      await invoke("export_audio_file", {
        request: {
          sourcePath: filePath,
          outputPath: selectedOut,
          bitDepth: exportBitDepth,
          rangeStartSeconds: rangeStart,
          rangeEndSeconds: rangeEnd,
          pitchSemitones: totalSemitones,
          speedMultiplier: speed,
          volumeMultiplier: dbToLinear(volume),
          bakePitch: exportBakePitch,
          bakeSpeed: exportBakeSpeed,
          bakeEq: exportBakeEq,
          bakeCompressor: exportBakeCompressor,
          bakeCuts: exportBakeCuts,
          eqBands: eqNodes.map(n => ({
            filterType: n.filterType,
            freq: n.freq,
            gainDb: n.gainDb,
            q: n.q,
            enabled: n.enabled && !isEqBypassed
          })),
          compStage1: {
            ...compStage1,
            enabled: compStage1.enabled && !isCompressorBypassed
          },
          compStage2: {
            ...compStage2,
            enabled: compStage2.enabled && !isCompressorBypassed
          },
          compRouting: compRouting,
          compParallelBlend: compParallelBlend,
          regions: regions.map(r => ({
            startSeconds: r.startTime,
            endSeconds: r.endTime,
            isLoop: r.isLoop,
            isCut: r.isCut,
            crossfadeMs: r.crossfadeMs ?? 5.0
          })),
          copyMetadata: exportCopyMetadata
        }
      });

      exportStatusMessage = `✓ Exported successfully to:\n${selectedOut.split("/").pop()}`;
      setTimeout(() => {
        if (!exportErrorMessage) {
          showExportModal = false;
        }
      }, 1800);
    } catch (err: any) {
      console.error("Export failed:", err);
      exportErrorMessage = `Export failed: ${err}`;
      exportStatusMessage = "";
    } finally {
      isExporting = false;
    }
  }

  // Show Control & Remotes Functions (Milestone 8)
  async function handleRemoteControlAction(payloadStr: string) {
    try {
      const parsed = typeof payloadStr === "string" ? JSON.parse(payloadStr) : payloadStr;
      const action = parsed.action || parsed.command;
      const data = parsed.data || {};

      switch (action) {
        case "play":
          if (!isPlaying) handlePlayPause();
          break;
        case "pause":
          if (isPlaying) handlePlayPause();
          break;
        case "play_pause":
          handlePlayPause();
          break;
        case "stop":
          handleStop();
          break;
        case "rewind":
          if (activeTab === "browser" && lastSelectedEntry && !lastSelectedEntry.is_dir && lastSelectedEntry.path !== filePath) {
            await loadAudioPath(lastSelectedEntry.path, "main", true);
            await invoke("play");
            isPlaying = true;
            return;
          }
          if (activeTab === "playlist" && selectedPlaylistIndex >= 0 && selectedPlaylistIndex < playlistItems.length) {
            const highlighted = playlistItems[selectedPlaylistIndex];
            if (highlighted && highlighted.path !== filePath) {
              await loadAudioPath(highlighted.path, "main", true);
              await invoke("play");
              isPlaying = true;
              return;
            }
          }
          await invoke("seek", { positionSeconds: 0.0 });
          currentTime = 0;
          break;
        case "next_track":
          if (activeTab === "browser" && filteredEntries.length > 0) {
            let currentIdx = filteredEntries.findIndex(e => selectedFilePaths.has(e.path));
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.min(filteredEntries.length - 1, currentIdx + 1);
            const targetEntry = filteredEntries[currentIdx];
            if (targetEntry) {
              selectedFilePaths.clear();
              selectedFilePaths.add(targetEntry.path);
              selectedFilePaths = selectedFilePaths;
              lastSelectedEntry = targetEntry;
              scrollSelectedBrowserItemIntoView();
              if (!targetEntry.is_dir) {
                await loadAudioPath(targetEntry.path, "main", true);
              }
            }
          } else if (playlistItems.length > 0) {
            let currentIdx = playlistItems.findIndex(p => p.path === filePath);
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.min(playlistItems.length - 1, currentIdx + 1);
            selectedPlaylistIndex = currentIdx;
            scrollSelectedPlaylistItemIntoView();
            const target = playlistItems[currentIdx];
            if (target) {
              await loadAudioPath(target.path, "main", true);
            }
          }
          break;
        case "prev_track":
          if (activeTab === "browser" && filteredEntries.length > 0) {
            let currentIdx = filteredEntries.findIndex(e => selectedFilePaths.has(e.path));
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.max(0, currentIdx - 1);
            const targetEntry = filteredEntries[currentIdx];
            if (targetEntry) {
              selectedFilePaths.clear();
              selectedFilePaths.add(targetEntry.path);
              selectedFilePaths = selectedFilePaths;
              lastSelectedEntry = targetEntry;
              scrollSelectedBrowserItemIntoView();
              if (!targetEntry.is_dir) {
                await loadAudioPath(targetEntry.path, "main", true);
              }
            }
          } else if (playlistItems.length > 0) {
            let currentIdx = playlistItems.findIndex(p => p.path === filePath);
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.max(0, currentIdx - 1);
            selectedPlaylistIndex = currentIdx;
            scrollSelectedPlaylistItemIntoView();
            const target = playlistItems[currentIdx];
            if (target) {
              await loadAudioPath(target.path, "main", true);
            }
          }
          break;
        case "select_track":
          if (typeof data.index === "number" && data.index >= 0 && data.index < playlistItems.length) {
            selectedPlaylistIndex = data.index;
            scrollSelectedPlaylistItemIntoView();
            const target = playlistItems[data.index];
            if (target) {
              await loadAudioPath(target.path, "main", true);
            }
          }
          break;
        case "next_marker":
          jumpToNextMarker();
          break;
        case "prev_marker":
          jumpToPrevMarker();
          break;
        case "add_marker":
          addMarker();
          break;
        case "pitch_up":
          pitch = Math.min(24, pitch + 1);
          updatePitchEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "pitch_down":
          pitch = Math.max(-24, pitch - 1);
          updatePitchEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "adjust_pitch":
          if (typeof data.delta === "number") {
            pitch = Math.max(-24, Math.min(24, pitch + data.delta));
            updatePitchEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "set_pitch":
          if (typeof data.semitones === "number") {
            pitch = Math.max(-24, Math.min(24, data.semitones));
            updatePitchEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "volume_up":
          volume = Math.min(6, volume + 1);
          updateVolumeEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "volume_down":
          volume = Math.max(-60, volume - 1);
          updateVolumeEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "adjust_volume":
          if (typeof data.delta === "number") {
            volume = Math.max(-60, Math.min(6, volume + data.delta));
            updateVolumeEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "set_volume":
          if (typeof data.db === "number") {
            volume = Math.max(-60, Math.min(6, data.db));
            updateVolumeEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "speed_up":
          speed = Math.min(4.0, parseFloat((speed + 0.05).toFixed(2)));
          updateSpeedEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "speed_down":
          speed = Math.max(0.25, parseFloat((speed - 0.05).toFixed(2)));
          updateSpeedEngine();
          saveCurrentTrackProfile(filePath);
          break;
        case "adjust_speed":
          if (typeof data.delta === "number") {
            speed = Math.max(0.25, Math.min(4.0, parseFloat((speed + data.delta).toFixed(2))));
            updateSpeedEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "set_speed":
          if (typeof data.speed === "number") {
            speed = Math.max(0.25, Math.min(4.0, parseFloat(data.speed.toFixed(2))));
            updateSpeedEngine();
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "toggle_loop":
          handleLoopHotkey();
          break;
        case "toggle_cut":
          handleCutHotkey();
          break;
        default:
          break;
      }
    } catch (e) {
      console.error("Error processing remote control action:", e);
    }
  }

  function handleMidiEvent(payloadStr: string) {
    try {
      const parsed = typeof payloadStr === "string" ? JSON.parse(payloadStr) : payloadStr;
      if (parsed.type === "note_on") {
        const note = parsed.note;
        if (note === 60) handlePlayPause(); // C4 = Play/Pause
        else if (note === 62) { // D4 = Rewind / Play highlighted
          handleRemoteControlAction(JSON.stringify({ action: "rewind" }));
        }
        else if (note === 64) jumpToNextMarker(); // E4 = Next Marker
        else if (note === 65) jumpToPrevMarker(); // F4 = Prev Marker
        else if (note === 67) addMarker(); // G4 = Add Marker
        else if (note === 69) handleLoopHotkey(); // A4 = Loop Toggle
        else if (note === 71) handleCutHotkey(); // B4 = Cut Toggle
      } else if (parsed.type === "cc") {
        const cc = parsed.cc;
        const val = parsed.value;
        if (cc === 7) { // CC 7 = Volume (0-127 -> -60dB to +6dB)
          const norm = val / 127.0;
          volume = parseFloat((-60 + norm * 66).toFixed(1));
          updateVolumeEngine();
          saveCurrentTrackProfile(filePath);
        } else if (cc === 1) { // CC 1 = Speed Mod (0-127 -> 0.5x to 2.0x)
          const norm = val / 127.0;
          speed = parseFloat((0.5 + norm * 1.5).toFixed(2));
          updateSpeedEngine();
          saveCurrentTrackProfile(filePath);
        }
      }
    } catch (e) {
      console.error("Error processing MIDI event:", e);
    }
  }

  async function openRemoteSettingsModal() {
    await refreshMidiPorts();
    showRemoteSettingsModal = true;
  }

  async function refreshMidiPorts() {
    try {
      midiPorts = await invoke("list_midi_devices");
      if (midiPorts.length > 0 && !selectedMidiPort) {
        selectedMidiPort = midiPorts[0];
      }
    } catch (e) {
      console.error("Failed to list MIDI ports:", e);
    }
  }

  async function connectToMidiPort(portName: string) {
    if (!portName) return;
    try {
      const connected = await invoke("connect_midi_device", { deviceName: portName });
      midiStatusMessage = `✓ Connected to: ${connected}`;
    } catch (e: any) {
      midiStatusMessage = `Connection failed: ${e}`;
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
        saveCurrentTrackProfile(filePath);
      }
    } catch (err) {
      alert("Failed to associate PDF: " + err);
    }
  }

  function clearPdfChart() {
    pdfChartPath = "";
    pdfChartName = "";
    localStorage.removeItem("th_pdf_path");
    saveCurrentTrackProfile(filePath);
  }

  async function openPdfInExternalViewer() {
    if (!pdfChartPath) return;
    try {
      await invoke("open_file_external", { path: pdfChartPath });
    } catch (err) {
      alert("Failed to open PDF in floating window: " + err);
    }
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

  function sliderValToZoom(val: number): number {
    if (maxZoom <= 1.0) return 1.0;
    const t = Math.max(0, Math.min(1000, val)) / 1000;
    return Math.exp(Math.log(1.0) + t * (Math.log(maxZoom) - Math.log(1.0)));
  }

  function zoomToSliderVal(z: number): number {
    if (maxZoom <= 1.0 || z <= 1.0) return 0;
    const t = (Math.log(z) - Math.log(1.0)) / (Math.log(maxZoom) - Math.log(1.0));
    return Math.max(0, Math.min(1000, t * 1000));
  }

  function setZoom(newZoom: number) {
    zoom = Math.max(1.0, Math.min(maxZoom, newZoom));
    zoomSliderVal = zoomToSliderVal(zoom);
    invalidateWaveformCaches();
  }

  // Dynamic Peak Slice & Waveform updater (Fast Synchronous Slicing with Background Sample Cache)
  function updateVisiblePeaks() {
    const track = getActiveTrack();
    if (!track || !mainCanvas || duration === 0) {
      visibleSamples = [];
      visibleSampleFrames = 0;
      drawMainWaveform();
      drawOverviewWaveform();
      return;
    }

    const rect = mainCanvas.getBoundingClientRect();
    const numPoints = Math.max(100, Math.floor(rect.width));

    const totalTrackFrames = duration * sampleRate;
    const windowWidth = 1.0 / zoom;
    
    let startProgress = 0;
    if (zoom > 1.001) {
      const halfWindow = windowWidth / 2;
      startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - halfWindow));
    }
    const endProgress = Math.min(1.0, startProgress + windowWidth);

    const startFrame = Math.floor(startProgress * totalTrackFrames);
    const endFrame = Math.min(totalTrackFrames, Math.floor(endProgress * totalTrackFrames));
    const visibleFrames = Math.max(1, endFrame - startFrame);
    visibleSampleFrames = visibleFrames;

    // If visibleFrames <= 3000: Deep zoom / sample level
    if (visibleFrames <= 3000) {
      if (
        localSampleCache.samples.length > 0 &&
        startFrame >= localSampleCache.startFrame &&
        endFrame <= localSampleCache.endFrame
      ) {
        const offset = startFrame - localSampleCache.startFrame;
        visibleSamples = localSampleCache.samples.slice(offset, offset + visibleFrames);
      } else {
        // Use slice from pyramid temporarily
        const pyramid = track.pyramidPeaks;
        if (pyramid && pyramid.length > 0) {
          const startIdx = Math.floor(startProgress * pyramid.length);
          const endIdx = Math.max(startIdx + 1, Math.floor(endProgress * pyramid.length));
          visibleSamples = pyramid.slice(startIdx, endIdx);
        }

        // Fetch sample window in background without blocking UI
        if (!isFetchingRawChunk) {
          isFetchingRawChunk = true;
          const fetchPadding = Math.max(6000, visibleFrames * 4);
          const fetchStart = Math.max(0, startFrame - Math.floor(fetchPadding / 2));
          const fetchCount = Math.min(totalTrackFrames - fetchStart, visibleFrames + fetchPadding);
          const reqId = ++currentRawSampleReqId;
          const reqPath = filePath;

          invoke("get_raw_samples", { startFrame: fetchStart, count: fetchCount })
            .then((res: any) => {
              if (reqId !== currentRawSampleReqId || reqPath !== filePath) {
                return; // Discard stale request after rapid track switch
              }
              const samples: number[] = res || [];
              localSampleCache = {
                startFrame: fetchStart,
                endFrame: fetchStart + samples.length,
                samples
              };
              isFetchingRawChunk = false;
              updateVisiblePeaks();
            })
            .catch(err => {
              console.error("Failed to fetch raw samples", err);
              isFetchingRawChunk = false;
            });
        }
      }
    } else {
      // Slicing from pyramidPeaks (32,768 precalculated single samples) synchronously in JS in 0.002ms!
      const pyramid = track.pyramidPeaks;
      if (pyramid && pyramid.length > 0) {
        const startIdx = Math.floor(startProgress * pyramid.length);
        const endIdx = Math.max(startIdx + 1, Math.floor(endProgress * pyramid.length));
        const sliceLen = endIdx - startIdx;

        if (sliceLen <= numPoints) {
          visibleSamples = pyramid.slice(startIdx, endIdx);
        } else {
          const downsampled: number[] = new Array(numPoints);
          const step = sliceLen / numPoints;
          for (let i = 0; i < numPoints; i++) {
            const bStart = Math.floor(startIdx + i * step);
            const bEnd = Math.min(pyramid.length, Math.max(bStart + 1, Math.floor(startIdx + (i + 1) * step)));
            let maxAbs = 0;
            let bestVal = 0;
            for (let j = bStart; j < bEnd; j++) {
              const val = pyramid[j];
              const abs = Math.abs(val);
              if (abs > maxAbs) {
                maxAbs = abs;
                bestVal = val;
              }
            }
            downsampled[i] = bestVal;
          }
          visibleSamples = downsampled;
        }
      } else {
        visibleSamples = [];
      }
    }

    drawMainWaveform();
    drawOverviewWaveform();
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
    });
  }

  async function handleVolume(e: Event) {
    const target = e.target as HTMLInputElement;
    dbVolume = parseFloat(target.value);
    volumeLinear = dbToLinear(dbVolume);
    await invoke("set_volume", { volume: volumeLinear });
    saveCurrentTrackProfile(filePath);
  }

  function handleRewind() {
    lastScrolledMarkerId = null;
    currentTime = 0;
    progress = 0;
    invoke("seek", { seconds: 0 });
    updateVisiblePeaks();
    drawMainWaveform();
    drawOverviewWaveform();
    if (activeCenterTab === "pdf" && pdfContainer) {
      pdfContainer.scrollTo({ top: 0, behavior: "smooth" });
    }
  }

  // Panning & Dragging State
  let isPanning = false;
  let panStartX = 0;
  let panStartProgress = 0;
  let hasDraggedMain = false;

  let isDraggingOverview = false;
  let isDraggingSelectionEdge: "start" | "end" | null = null;
  let isDraggingRegionEdge: { id: string, edge: "start" | "end" } | null = null;

  function getHoveredEdge(clickX: number, width: number, startProgress: number, windowWidth: number): { type: "selection" | "region", edge: "start" | "end", regionId?: string } | null {
    const edgeThresholdPx = 8;

    // 1. Check timeSelection edges
    if (timeSelection && duration > 0) {
      const selStartX = ((timeSelection.start / duration - startProgress) / windowWidth) * width;
      const selEndX = ((timeSelection.end / duration - startProgress) / windowWidth) * width;
      if (Math.abs(clickX - selStartX) <= edgeThresholdPx) {
        return { type: "selection", edge: "start" };
      }
      if (Math.abs(clickX - selEndX) <= edgeThresholdPx) {
        return { type: "selection", edge: "end" };
      }
    }

    // 2. Check regions edges (reverse order so top/selected regions get priority)
    for (let i = regions.length - 1; i >= 0; i--) {
      const reg = regions[i];
      const regStartX = ((reg.startTime / duration - startProgress) / windowWidth) * width;
      const regEndX = ((reg.endTime / duration - startProgress) / windowWidth) * width;
      if (Math.abs(clickX - regStartX) <= edgeThresholdPx) {
        return { type: "region", edge: "start", regionId: reg.id };
      }
      if (Math.abs(clickX - regEndX) <= edgeThresholdPx) {
        return { type: "region", edge: "end", regionId: reg.id };
      }
    }

    return null;
  }

  function handleMainCanvasHover(e: MouseEvent) {
    if (!mainCanvas || isPanning || isShiftSelecting || isDraggingMarker || isDraggingSelectionEdge || isDraggingRegionEdge) return;
    const rect = mainCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const windowWidth = 1.0 / zoom;
    const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;

    const hovered = getHoveredEdge(clickX, rect.width, startProgress, windowWidth);
    if (hovered) {
      mainCanvas.style.cursor = "ew-resize";
    } else {
      mainCanvas.style.cursor = "default";
    }
  }

  // Main Waveform: Mouse Drag to Pan, click to seek, Shift-drag region select, and Ruler Marker Dragging
  function handleMainMouseDown(e: MouseEvent) {
    if (duration === 0 || !mainCanvas) return;

    const rect = mainCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const clickY = e.clientY - rect.top;

    const windowWidth = 1.0 / zoom;
    const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
    const endProgress = startProgress + windowWidth;

    // Check if clicked near a marker flag in the top ruler area (top 24px)
    if (clickY <= 24) {
      for (const m of markers) {
        const markerPct = m.time / duration;
        if (markerPct >= startProgress && markerPct <= endProgress) {
          const markerX = ((markerPct - startProgress) / windowWidth) * rect.width;
          if (clickX >= markerX - 6 && clickX <= markerX + 18) {
            if (e.shiftKey) {
              if (selectedMarkerIds.has(m.id)) {
                selectedMarkerIds.delete(m.id);
              } else {
                if (selectedMarkerIds.size >= 2) selectedMarkerIds.clear();
                selectedMarkerIds.add(m.id);
              }
              selectedMarkerIds = selectedMarkerIds;
              if (selectedMarkerIds.size === 2) {
                const arr = markers.filter(x => selectedMarkerIds.has(x.id)).sort((a, b) => a.time - b.time);
                timeSelection = { start: arr[0].time, end: arr[1].time };
              } else {
                timeSelection = null;
              }
              drawMainWaveform();
              return;
            }
            startMarkerPointerDrag(e, m, false, true);
            return;
          }
        }
      }
    }

    // Check if clicked near an edge of timeSelection or a Region to resize
    const edgeHit = getHoveredEdge(clickX, rect.width, startProgress, windowWidth);
    if (edgeHit) {
      if (edgeHit.type === "selection") {
        isDraggingSelectionEdge = edgeHit.edge;
        window.addEventListener("mousemove", handleMainMouseMove);
        window.addEventListener("mouseup", handleMainMouseUp);
        return;
      } else if (edgeHit.type === "region" && edgeHit.regionId) {
        isDraggingRegionEdge = { id: edgeHit.regionId, edge: edgeHit.edge };
        selectedRegionId = edgeHit.regionId;
        window.addEventListener("mousemove", handleMainMouseMove);
        window.addEventListener("mouseup", handleMainMouseUp);
        return;
      }
    }

    if (e.shiftKey) {
      isShiftSelecting = true;
      const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
      selectionDragStart = Math.max(0, Math.min(duration, (startProgress + clickPct * windowWidth) * duration));
      timeSelection = { start: selectionDragStart, end: selectionDragStart };
      drawMainWaveform();

      window.addEventListener("mousemove", handleMainMouseMove);
      window.addEventListener("mouseup", handleMainMouseUp);
      return;
    }

    // Normal click/pan: clear time selection if clicking plain waveform
    timeSelection = null;
    
    // Select region if clicked inside its span
    const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
    const clickTime = (startProgress + clickPct * windowWidth) * duration;
    const clickedRegion = regions.find(r => clickTime >= r.startTime && clickTime <= r.endTime);
    selectedRegionId = clickedRegion ? clickedRegion.id : null;

    isPanning = true;
    hasDraggedMain = false;
    panStartX = e.clientX;
    panStartProgress = progress;
    
    window.addEventListener("mousemove", handleMainMouseMove);
    window.addEventListener("mouseup", handleMainMouseUp);
  }

  function handleMainMouseMove(e: MouseEvent) {
    if (isDraggingMarker && draggingMarkerId != null && mainCanvas) {
      const rect = mainCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const targetProgress = Math.max(0, Math.min(1.0, startProgress + clickPct * windowWidth));
      const newTime = targetProgress * duration;

      const m = markers.find(x => x.id === draggingMarkerId);
      if (m) {
        m.time = newTime;
        markers = markers;
      }
      drawMainWaveform();
      drawOverviewWaveform();
      return;
    }

    if (isDraggingSelectionEdge && mainCanvas && timeSelection) {
      const rect = mainCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const currentDragTime = Math.max(0, Math.min(duration, (startProgress + clickPct * windowWidth) * duration));

      if (isDraggingSelectionEdge === "start") {
        timeSelection.start = Math.max(0, Math.min(timeSelection.end - 0.02, currentDragTime));
      } else {
        timeSelection.end = Math.min(duration, Math.max(timeSelection.start + 0.02, currentDragTime));
      }
      drawMainWaveform();
      return;
    }

    if (isDraggingRegionEdge && mainCanvas) {
      const rect = mainCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const currentDragTime = Math.max(0, Math.min(duration, (startProgress + clickPct * windowWidth) * duration));

      const reg = regions.find(r => r.id === isDraggingRegionEdge!.id);
      if (reg) {
        if (isDraggingRegionEdge.edge === "start") {
          reg.startTime = Math.max(0, Math.min(reg.endTime - 0.02, currentDragTime));
        } else {
          reg.endTime = Math.min(duration, Math.max(reg.startTime + 0.02, currentDragTime));
        }
        regions = [...regions];
      }
      drawMainWaveform();
      return;
    }

    if (isShiftSelecting && mainCanvas) {
      const rect = mainCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickPct = Math.max(0, Math.min(1.0, clickX / rect.width));
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const currentDragTime = Math.max(0, Math.min(duration, (startProgress + clickPct * windowWidth) * duration));
      timeSelection = {
        start: Math.min(selectionDragStart, currentDragTime),
        end: Math.max(selectionDragStart, currentDragTime)
      };
      drawMainWaveform();
      return;
    }

    if (!isPanning || !mainCanvas) return;
    const deltaX = e.clientX - panStartX;
    if (Math.abs(deltaX) > 3) {
      hasDraggedMain = true;
    }

    // Only allow panning if zoomed in
    if (zoom > 1.001) {
      const rect = mainCanvas.getBoundingClientRect();
      const width = rect.width;
      const windowWidth = 1.0 / zoom;
      const progressChange = -(deltaX / width) * windowWidth;
      let newProgress = panStartProgress + progressChange;
      const halfWindow = windowWidth / 2;
      newProgress = Math.max(halfWindow, Math.min(1.0 - halfWindow, newProgress));

      progress = newProgress;
      currentTime = progress * duration;

      updateVisiblePeaks();
      drawMainWaveform();
      drawOverviewWaveform();
    }
  }

  async function handleMainMouseUp(e: MouseEvent) {
    if (isDraggingMarker) {
      isDraggingMarker = false;
      draggingMarkerId = null;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);

      markers.sort((a, b) => a.time - b.time);
      markers = markers;
      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
      drawOverviewWaveform();
      if (activeCenterTab === "pdf") {
        renderPdfMarkerBadges();
      }
      return;
    }

    if (isDraggingSelectionEdge) {
      isDraggingSelectionEdge = null;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);
      drawMainWaveform();
      return;
    }

    if (isDraggingRegionEdge) {
      isDraggingRegionEdge = null;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);
      syncRegionsToEngine();
      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
      return;
    }

    if (isShiftSelecting) {
      isShiftSelecting = false;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);
      if (timeSelection && Math.abs(timeSelection.end - timeSelection.start) < 0.05) {
        timeSelection = null;
      }
      drawMainWaveform();
      return;
    }

    if (isPanning) {
      isPanning = false;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);

      if (!hasDraggedMain && mainCanvas) {
        // Simple Click: seek playhead instantly to clicked point!
        const rect = mainCanvas.getBoundingClientRect();
        const clickX = e.clientX - rect.left;
        const clickPct = Math.max(0, Math.min(1, clickX / rect.width));
        
        const windowWidth = 1.0 / zoom;
        let startProgress = 0;
        if (zoom > 1.001) {
          const halfWindow = windowWidth / 2;
          startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - halfWindow));
        }
        const targetProgress = Math.max(0, Math.min(1.0, startProgress + clickPct * windowWidth));
        const targetSeconds = targetProgress * duration;

        // Instant local update (0ms delay)
        progress = targetProgress;
        currentTime = targetSeconds;
        updateVisiblePeaks();

        // Background seek
        await invoke("seek", { seconds: targetSeconds });
      } else if (hasDraggedMain) {
        // Drag finished: sync audio engine playhead
        await invoke("seek", { seconds: progress * duration });
      }
    }
  }

  // Region Creation & Operations
  function createRegionFromSelectionOrMarkers(initialLoop = false, initialCut = false): Region | null {
    let start = 0;
    let end = 0;
    let name = `Region ${nextRegionId}`;

    if (timeSelection && Math.abs(timeSelection.end - timeSelection.start) > 0.05) {
      start = timeSelection.start;
      end = timeSelection.end;
    } else if (selectedMarkerIds.size === 2) {
      const arr = markers.filter(m => selectedMarkerIds.has(m.id)).sort((a, b) => a.time - b.time);
      if (arr.length === 2) {
        start = arr[0].time;
        end = arr[1].time;
        name = `${arr[0].name} – ${arr[1].name}`;
      }
    } else if (markers.length >= 2) {
      const preceding = [...markers].filter(m => m.time <= currentTime).sort((a, b) => b.time - a.time)[0];
      const following = [...markers].filter(m => m.time > currentTime).sort((a, b) => a.time - b.time)[0];
      if (preceding && following) {
        start = preceding.time;
        end = following.time;
        name = `${preceding.name} – ${following.name}`;
      }
    }

    if (end > start) {
      const newRegion: Region = {
        id: `reg_${Date.now()}_${nextRegionId++}`,
        name,
        startTime: start,
        endTime: end,
        isLoop: initialLoop,
        isCut: initialCut,
        color: initialCut ? "#ff453a" : initialLoop ? "#30d158" : "#0a84ff"
      };
      regions = [...regions, newRegion];
      selectedRegionId = newRegion.id;
      timeSelection = null;
      selectedMarkerIds.clear();
      selectedMarkerIds = selectedMarkerIds;
      syncRegionsToEngine();
      return newRegion;
    }
    return null;
  }

  function handleCutHotkey() {
    if (selectedRegionId) {
      const reg = regions.find(r => r.id === selectedRegionId);
      if (reg) {
        toggleRegionCut(reg);
        return;
      }
    }
    createRegionFromSelectionOrMarkers(false, true);
  }

  function handleLoopHotkey() {
    if (selectedRegionId) {
      const reg = regions.find(r => r.id === selectedRegionId);
      if (reg) {
        toggleRegionLoop(reg);
        return;
      }
    }
    createRegionFromSelectionOrMarkers(true, false);
  }

  function toggleRegionLoop(region: Region) {
    region.isLoop = !region.isLoop;
    if (region.isLoop) {
      region.isCut = false; // Cannot be loop and cut simultaneously
    }
    regions = [...regions];
    syncRegionsToEngine();
  }

  function toggleRegionCut(region: Region) {
    region.isCut = !region.isCut;
    if (region.isCut) {
      region.isLoop = false; // Cannot be cut and loop simultaneously
    }
    regions = [...regions];
    syncRegionsToEngine();
  }

  function deleteRegion(id: string) {
    regions = regions.filter(r => r.id !== id);
    if (selectedRegionId === id) selectedRegionId = null;
    syncRegionsToEngine();
  }

  function startRenameRegion(region: Region) {
    editingRegionId = region.id;
    editingRegionName = region.name;
  }

  function saveRenameRegion(region: Region) {
    if (editingRegionName && editingRegionName.trim()) {
      region.name = editingRegionName.trim();
      regions = [...regions];
      syncRegionsToEngine();
      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
    }
    editingRegionId = null;
  }

  function openRegionContextMenu(e: MouseEvent, region: Region) {
    e.preventDefault();
    contextMenuRegion = region;
    selectedRegionId = region.id;
    regionContextMenuX = e.clientX;
    regionContextMenuY = e.clientY;
    showRegionContextMenu = true;
  }

  // Mouse wheel zoom on Main Waveform
  function handleMainWheel(e: WheelEvent) {
    if (duration === 0) return;
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.15 : 0.85;
    let newZoom = zoom * factor;
    newZoom = Math.max(1.0, Math.min(maxZoom, newZoom));
    
    // Soft snap to 15s notch if scrolling close to it
    if (Math.abs(newZoom - target15sZoom) < target15sZoom * 0.05) {
      newZoom = target15sZoom;
    }

    if (newZoom !== zoom) {
      setZoom(newZoom);
      updateVisiblePeaks();
    }
  }

  function handleZoomSliderInput(e: Event) {
    const inputVal = parseFloat((e.target as HTMLInputElement).value);
    let newZoom = sliderValToZoom(inputVal);
    
    // Soft snap to 15s if close
    if (Math.abs(newZoom - target15sZoom) / target15sZoom < 0.08) {
      newZoom = target15sZoom;
      zoomSliderVal = zoomToSliderVal(target15sZoom);
    } else {
      zoomSliderVal = inputVal;
    }

    zoom = newZoom;
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
    const canvas = overviewCanvas;
    if (!canvas || duration === 0) return;
    const rect = canvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const pct = Math.max(0, Math.min(1.0, clickX / rect.width));

    progress = pct;
    currentTime = progress * duration;

    updateVisiblePeaks();
    invoke("seek", { seconds: currentTime });
  }

  // Markers
  function addMarker() {
    if (duration === 0) return;
    const colorIndex = markers.length % MARKER_COLORS.length;
    const newMarker: Marker = {
      id: nextMarkerId++,
      name: `Marker ${nextMarkerId - 1}`,
      time: currentTime,
      color: MARKER_COLORS[colorIndex]
    };
    markers = [...markers, newMarker].sort((a, b) => a.time - b.time);
    saveCurrentTrackProfile(filePath);
    invalidateWaveformCaches();
    drawMainWaveform();
    drawOverviewWaveform();
  }

  function deleteMarker(id: number) {
    markers = markers.filter(m => m.id !== id);
    saveCurrentTrackProfile(filePath);
    invalidateWaveformCaches();
    drawMainWaveform();
    drawOverviewWaveform();
  }

  function openColorPalette(e: MouseEvent, marker: Marker) {
    e.stopPropagation();
    colorPaletteMarker = marker;
    const target = e.currentTarget as HTMLElement;
    if (target) {
      const rect = target.getBoundingClientRect();
      const popoverHeight = 85;
      if (rect.bottom + popoverHeight > window.innerHeight) {
        colorPaletteY = Math.max(10, rect.top - popoverHeight - 4);
      } else {
        colorPaletteY = rect.bottom + 6;
      }
      colorPaletteX = Math.max(10, Math.min(window.innerWidth - 180, rect.left - 40));
    }
  }

  function setMarkerColor(marker: Marker, color: string) {
    const markerName = marker.name.trim();
    marker.color = color;
    markers = [...markers];

    // Propagate color project-wide to all takes sharing this landmark name
    const store = getProfilesStore();
    let hasChanges = false;
    for (const trackPath in store) {
      if (store[trackPath]?.markers && Array.isArray(store[trackPath].markers)) {
        for (const m of store[trackPath].markers) {
          if (m.name.trim().toLowerCase() === markerName.toLowerCase()) {
            m.color = color;
            hasChanges = true;
          }
        }
      }
    }
    if (hasChanges) {
      localStorage.setItem("th_track_profiles", JSON.stringify(store));
    }

    saveCurrentTrackProfile(filePath);
    invalidateWaveformCaches();
    drawMainWaveform();
    drawOverviewWaveform();
    if (activeCenterTab === "pdf") {
      renderPdfMarkerBadges();
    }
    colorPaletteMarker = null;
  }

  function startRenameMarker(marker: Marker) {
    editingMarkerId = marker.id;
    editingMarkerName = marker.name;
  }

  function saveRenameMarker(marker: Marker) {
    if (editingMarkerId === marker.id) {
      const oldName = marker.name.trim();
      const newName = editingMarkerName.trim() || marker.name;
      marker.name = newName;
      editingMarkerId = null;
      markers = [...markers];

      // Update this landmark name project-wide across all saved profiles!
      const store = getProfilesStore();
      let hasChanges = false;
      for (const trackPath in store) {
        if (store[trackPath]?.markers && Array.isArray(store[trackPath].markers)) {
          for (const m of store[trackPath].markers) {
            if (m.name.trim().toLowerCase() === oldName.toLowerCase()) {
              m.name = newName;
              hasChanges = true;
            }
          }
        }
      }
      if (hasChanges) {
        localStorage.setItem("th_track_profiles", JSON.stringify(store));
      }

      saveCurrentTrackProfile(filePath);
      drawMainWaveform();
      drawOverviewWaveform();
      if (activeCenterTab === "pdf") {
        renderPdfMarkerBadges();
      }
    }
  }

  function cancelRenameMarker() {
    editingMarkerId = null;
  }

  function deleteProjectLandmark(name: string) {
    const targetName = name.trim().toLowerCase();
    markers = markers.filter(m => m.name.trim().toLowerCase() !== targetName);
    const store = getProfilesStore();
    for (const trackPath in store) {
      if (store[trackPath]?.markers && Array.isArray(store[trackPath].markers)) {
        store[trackPath].markers = store[trackPath].markers.filter((m: Marker) => m.name.trim().toLowerCase() !== targetName);
      }
    }
    localStorage.setItem("th_track_profiles", JSON.stringify(store));
    saveCurrentTrackProfile(filePath);
    drawMainWaveform();
    drawOverviewWaveform();
    if (activeCenterTab === "pdf") {
      renderPdfMarkerBadges();
    }
  }

  function seekToMarker(time: number) {
    lastScrolledMarkerId = null;
    if (duration > 0) {
      currentTime = time;
      progress = time / duration;
      updateVisiblePeaks();
    }
    invoke("seek", { seconds: time });

    // Instantly scroll PDF to this marker if in PDF tab
    if (activeCenterTab === "pdf" && pdfContainer) {
      const match = markers.find(m => Math.abs(m.time - time) < 0.2);
      let anchor = match?.pdfAnchor;
      if (!anchor && activeTrackMode === "alternate" && mainTrack) {
        const mainMarkers = getProfilesStore()[mainTrack.path]?.markers || [];
        const mm = mainMarkers.find(m => m.name.trim().toLowerCase() === match?.name.trim().toLowerCase());
        anchor = mm?.pdfAnchor;
      }
      if (anchor) {
        const markerPage = anchor.page;
        const card = pdfContainer.querySelector(`.pdf-page-card[data-page-num="${markerPage}"]`) as HTMLElement;
        if (card) {
          const containerRect = pdfContainer.getBoundingClientRect();
          const cardRect = card.getBoundingClientRect();
          const cardTopRelativeToContainer = (cardRect.top - containerRect.top) + pdfContainer.scrollTop;
          const markerYWithinCard = card.clientHeight * anchor.yPct;
          const targetScrollTop = Math.max(0, cardTopRelativeToContainer + markerYWithinCard - 12);
          pdfContainer.scrollTo({
            top: targetScrollTop,
            behavior: "smooth"
          });
        }
      }
    }
  }

  function jumpToPrevMarker() {
    if (markers.length === 0) {
      if (duration > 0) {
        currentTime = 0;
        progress = 0;
        updateVisiblePeaks();
      }
      invoke("seek", { seconds: 0 });
      return;
    }
    const prev = [...markers]
      .reverse()
      .find(m => m.time < currentTime - 0.5);
    if (prev) {
      seekToMarker(prev.time);
    } else {
      if (duration > 0) {
        currentTime = 0;
        progress = 0;
        updateVisiblePeaks();
      }
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

  // Draw Main Waveform (Full Height Mono, Dynamic Time Ruler, Continuous Single Line)
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
    if (!track || visibleSamples.length === 0 || duration === 0) {
      ctx.fillStyle = "#888888";
      ctx.font = "13px sans-serif";
      ctx.fillText("No active track loaded (Drag & Drop file to load)", width / 2 - 140, height / 2 + 4);
      ctx.restore();
      return;
    }

    const windowWidth = 1.0 / zoom;
    let startProgress = 0;
    if (zoom > 1.001) {
      const halfWindow = windowWidth / 2;
      startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - halfWindow));
    }
    const endProgress = Math.min(1.0, startProgress + windowWidth);

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

    // 4. Render Time Selection & Regions
    if (timeSelection && duration > 0) {
      const selStartPct = timeSelection.start / duration;
      const selEndPct = timeSelection.end / duration;
      const x1 = Math.max(0, ((selStartPct - startProgress) / windowWidth) * width);
      const x2 = Math.min(width, ((selEndPct - startProgress) / windowWidth) * width);
      if (x2 > x1) {
        ctx.fillStyle = "rgba(10, 132, 255, 0.22)";
        ctx.fillRect(x1, rulerHeight, x2 - x1, height - rulerHeight);
        ctx.strokeStyle = "#0a84ff";
        ctx.lineWidth = 1.0;
        ctx.strokeRect(x1, rulerHeight, x2 - x1, height - rulerHeight);

        // Selection Edge Draggable Grab Handles
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(x1 - 2, height / 2 - 12, 4, 24);
        ctx.fillStyle = "#0a84ff";
        ctx.fillRect(x1 - 1, height / 2 - 8, 2, 16);

        ctx.fillStyle = "#ffffff";
        ctx.fillRect(x2 - 2, height / 2 - 12, 4, 24);
        ctx.fillStyle = "#0a84ff";
        ctx.fillRect(x2 - 1, height / 2 - 8, 2, 16);

        // Top Selection Duration Badge
        const selSec = (timeSelection.end - timeSelection.start).toFixed(2);
        ctx.fillStyle = "#0a84ff";
        ctx.font = "bold 9px monospace";
        ctx.fillText(`⟷ ${selSec}s`, x1 + 6, rulerHeight + 12);
      }
    }

    for (const reg of regions) {
      const regStartPct = reg.startTime / duration;
      const regEndPct = reg.endTime / duration;
      const x1 = ((regStartPct - startProgress) / windowWidth) * width;
      const x2 = ((regEndPct - startProgress) / windowWidth) * width;

      if (x2 > 0 && x1 < width) {
        const renderX1 = Math.max(0, x1);
        const renderX2 = Math.min(width, x2);
        const regWidth = renderX2 - renderX1;
        const handleColor = reg.isCut ? "#ff453a" : reg.isLoop ? "#30d158" : "#0a84ff";

        if (reg.isCut) {
          // Grayed out cut region with red-tinted diagonal hazard hatch
          ctx.fillStyle = "rgba(18, 20, 24, 0.75)";
          ctx.fillRect(renderX1, rulerHeight, regWidth, height - rulerHeight);
          
          ctx.save();
          ctx.beginPath();
          ctx.rect(renderX1, rulerHeight, regWidth, height - rulerHeight);
          ctx.clip();
          ctx.strokeStyle = "rgba(255, 69, 58, 0.25)";
          ctx.lineWidth = 1.5;
          for (let hx = renderX1 - height; hx < renderX2 + height; hx += 16) {
            ctx.beginPath();
            ctx.moveTo(hx, rulerHeight);
            ctx.lineTo(hx + height, height);
            ctx.stroke();
          }
          ctx.restore();

          // Solid vertical boundary lines (1px)
          ctx.strokeStyle = "#ff453a";
          ctx.lineWidth = 1.0;
          ctx.beginPath();
          ctx.moveTo(renderX1, rulerHeight);
          ctx.lineTo(renderX1, height);
          ctx.moveTo(renderX2, rulerHeight);
          ctx.lineTo(renderX2, height);
          ctx.stroke();

          // Cut Header Badge
          ctx.fillStyle = "#ff453a";
          ctx.font = "bold 9px sans-serif";
          ctx.fillText(`✂ CUT: ${reg.name}`, renderX1 + 6, rulerHeight + 13);
        } else if (reg.isLoop) {
          // Highlighted green looped region
          ctx.fillStyle = "rgba(48, 209, 88, 0.16)";
          ctx.fillRect(renderX1, rulerHeight, regWidth, height - rulerHeight);
          
          // Bracket borders (1px)
          ctx.strokeStyle = "#30d158";
          ctx.lineWidth = 1.0;
          ctx.beginPath();
          ctx.moveTo(renderX1, rulerHeight);
          ctx.lineTo(renderX1, height);
          ctx.moveTo(renderX2, rulerHeight);
          ctx.lineTo(renderX2, height);
          ctx.stroke();

          // Loop Header Badge
          ctx.fillStyle = "#30d158";
          ctx.font = "bold 9px sans-serif";
          ctx.fillText(`🔁 LOOP: ${reg.name}`, renderX1 + 6, rulerHeight + 13);
        } else {
          // Standard region
          ctx.fillStyle = "rgba(10, 132, 255, 0.12)";
          ctx.fillRect(renderX1, rulerHeight, regWidth, height - rulerHeight);
          ctx.strokeStyle = "rgba(10, 132, 255, 0.5)";
          ctx.lineWidth = 1;
          ctx.strokeRect(renderX1, rulerHeight, regWidth, height - rulerHeight);
          ctx.fillStyle = "#64d2ff";
          ctx.font = "9px sans-serif";
          ctx.fillText(`REGION: ${reg.name}`, renderX1 + 6, rulerHeight + 13);
        }

        // Draggable Edge Grab Handles for Region Left & Right Edges
        ctx.fillStyle = handleColor;
        ctx.fillRect(renderX1 - 2, height / 2 - 10, 4, 20);
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(renderX1 - 1, height / 2 - 6, 2, 12);

        ctx.fillStyle = handleColor;
        ctx.fillRect(renderX2 - 2, height / 2 - 10, 4, 20);
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(renderX2 - 1, height / 2 - 6, 2, 12);
      }
    }

    // 5. Translucent Yellow Compressor Threshold Overlay
    const halfHeight = Math.floor(height / 2);
    const maxAmplitude = (height / 2) - rulerHeight - 4;

    if (compressorThreshold < -0.01) {
      const threshLinear = Math.pow(10, compressorThreshold / 20.0);
      const topThreshY = halfHeight - threshLinear * maxAmplitude;
      const botThreshY = halfHeight + threshLinear * maxAmplitude;

      // Dotted threshold boundary lines only (no solid/translucent color fill)
      ctx.strokeStyle = "rgba(255, 214, 10, 0.75)";
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      
      ctx.beginPath();
      ctx.moveTo(0, topThreshY);
      ctx.lineTo(width, topThreshY);
      ctx.moveTo(0, botThreshY);
      ctx.lineTo(width, botThreshY);
      ctx.stroke();
      ctx.setLineDash([]);

      ctx.fillStyle = "rgba(255, 214, 10, 0.85)";
      ctx.font = "8px monospace";
      ctx.fillText(`${compressorThreshold.toFixed(1)} dB THRESHOLD`, 6, Math.max(rulerHeight + 10, topThreshY - 3));
    }

    // Zero-Crossing Baseline (Centered Vertically)
    ctx.strokeStyle = "rgba(59, 153, 252, 0.35)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, halfHeight + 0.5);
    ctx.lineTo(width, halfHeight + 0.5);
    ctx.stroke();

    // 0 dB center baseline label
    ctx.fillStyle = "rgba(59, 153, 252, 0.4)";
    ctx.font = "8px monospace";
    ctx.fillText("0 dB", 4, halfHeight - 3);

    // 6. Mono Continuous Single Line Waveform (Ghost + Compressed or Standard)
    if (visibleSamples.length > 0) {
      const numSamples = visibleSamples.length;
      const step = width / (numSamples - 1 || 1);
      const isCompActive = compressorThreshold < -0.01 && compressorRatio > 1.01;

      if (isCompActive) {
        // A. Draw original uncompressed waveform as translucent white ghost
        ctx.strokeStyle = "rgba(255, 255, 255, 0.22)";
        ctx.lineWidth = 1.0;
        ctx.beginPath();
        for (let i = 0; i < numSamples; i++) {
          const x = i * step;
          const y = halfHeight - visibleSamples[i] * maxAmplitude;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();

        // B. Draw resulting compressed waveform in vibrant theme colors
        const threshLinear = Math.pow(10, compressorThreshold / 20.0);
        const makeupLinear = Math.pow(10, compressorMakeup / 20.0);
        
        ctx.strokeStyle = "#3b99fc";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < numSamples; i++) {
          const rawAmp = visibleSamples[i];
          const absAmp = Math.abs(rawAmp);
          const sign = rawAmp >= 0 ? 1 : -1;
          
          let compAmp = absAmp;
          if (absAmp > threshLinear) {
            const rawDb = 20.0 * Math.log10(absAmp);
            const excessDb = rawDb - compressorThreshold;
            const compressedDb = compressorThreshold + (excessDb / compressorRatio);
            compAmp = Math.pow(10, compressedDb / 20.0);
          }
          compAmp *= makeupLinear;
          const y = halfHeight - (sign * compAmp) * maxAmplitude;
          const x = i * step;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();
      } else {
        // Standard normal continuous waveform
        ctx.strokeStyle = "#3b99fc";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < numSamples; i++) {
          const x = i * step;
          const y = halfHeight - visibleSamples[i] * maxAmplitude;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();
      }

      // Progressive Sample Node Squares (RX style when zoomed into <= 400 frames on screen)
      if (visibleSampleFrames <= 400) {
        let nodeSize = 2.5;
        if (visibleSampleFrames <= 40) {
          nodeSize = 6.0;
        } else if (visibleSampleFrames <= 150) {
          nodeSize = 4.0;
        }

        ctx.fillStyle = "#8fc4fa";
        for (let i = 0; i < numSamples; i++) {
          const x = i * step;
          const y = halfHeight - visibleSamples[i] * maxAmplitude;
          ctx.fillRect(x - nodeSize / 2, y - nodeSize / 2, nodeSize, nodeSize);
          if (nodeSize >= 4) {
            ctx.strokeStyle = "#0c151e";
            ctx.lineWidth = 1;
            ctx.strokeRect(x - nodeSize / 2, y - nodeSize / 2, nodeSize, nodeSize);
          }
        }
      }
    }

    // 5. Markers & Regions
    for (const marker of markers) {
      const markerPct = marker.time / duration;
      if (markerPct >= startProgress && markerPct <= endProgress) {
        const markerX = ((markerPct - startProgress) / windowWidth) * width;
        const color = marker.color || "#ff9500";
        const isMarkerSelected = selectedMarkerIds.has(marker.id);

        if (isMarkerSelected) {
          // Luminous selection aura for selected markers
          ctx.strokeStyle = "#ffffff";
          ctx.lineWidth = 2.5;
          ctx.beginPath();
          ctx.moveTo(markerX, rulerHeight);
          ctx.lineTo(markerX, height);
          ctx.stroke();

          ctx.fillStyle = "#ffffff";
          ctx.fillRect(markerX - 2, 0, 12, rulerHeight);
        }

        // Vertical Marker Line (1px)
        ctx.strokeStyle = isMarkerSelected ? "#ffffff" : color; 
        ctx.lineWidth = 1.0;
        ctx.beginPath();
        ctx.moveTo(markerX, rulerHeight);
        ctx.lineTo(markerX, height);
        ctx.stroke();
        
        // Ruler Top Flag
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(markerX, 0);
        ctx.lineTo(markerX + 8, 0);
        ctx.lineTo(markerX + 8, rulerHeight - 3);
        ctx.lineTo(markerX, rulerHeight);
        ctx.closePath();
        ctx.fill();

        // Label Badge
        ctx.fillStyle = isMarkerSelected ? "#ffffff" : color;
        ctx.font = "bold 9px sans-serif";
        ctx.fillText(marker.name, markerX + 4, rulerHeight + 12);
      }
    }

    // 5.5 Live Ghost Marker Preview (during active drag over waveform)
    if (dragWaveformPreviewMarker && dragWaveformPreviewX !== null) {
      const dropX = dragWaveformPreviewX - rect.left;
      if (dropX >= 0 && dropX <= width) {
        const color = dragWaveformPreviewMarker.color || "#ff9500";
        ctx.save();
        
        // Dashed Ghost Vertical Marker Line (1px)
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.0;
        ctx.setLineDash([4, 3]);
        ctx.beginPath();
        ctx.moveTo(dropX, rulerHeight);
        ctx.lineTo(dropX, height);
        ctx.stroke();
        ctx.setLineDash([]);

        // Ghost Ruler Top Flag
        ctx.fillStyle = color;
        ctx.globalAlpha = 0.9;
        ctx.beginPath();
        ctx.moveTo(dropX, 0);
        ctx.lineTo(dropX + 8, 0);
        ctx.lineTo(dropX + 8, rulerHeight - 3);
        ctx.lineTo(dropX, rulerHeight);
        ctx.closePath();
        ctx.fill();

        // Ghost Label Badge
        ctx.fillStyle = color;
        ctx.font = "bold 9px sans-serif";
        ctx.fillText(dragWaveformPreviewMarker.name, dropX + 4, rulerHeight + 12);
        ctx.restore();
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

    if (!overviewBaseCanvas) {
      overviewBaseCanvas = document.createElement("canvas");
    }
    const canvasWidth = Math.floor(rect.width * dpr);
    const canvasHeight = Math.floor(rect.height * dpr);
    if (overviewBaseCanvas.width !== canvasWidth || overviewBaseCanvas.height !== canvasHeight) {
      overviewBaseCanvas.width = canvasWidth;
      overviewBaseCanvas.height = canvasHeight;
      isOverviewBaseDirty = true;
    }

    if (isOverviewBaseDirty) {
      const bCtx = overviewBaseCanvas.getContext("2d");
      if (bCtx) {
        bCtx.clearRect(0, 0, canvasWidth, canvasHeight);
        bCtx.save();
        bCtx.scale(dpr, dpr);

        const isActive = activeTrackMode === mode;
        bCtx.fillStyle = isActive ? "#262626" : "#1a1a1a";
        bCtx.fillRect(0, 0, width, height);

        if (!track) {
          bCtx.fillStyle = "#666666";
          bCtx.font = "10px sans-serif";
          bCtx.fillText(`Empty [Double click file in browser to load as ${mode.toUpperCase()}]`, 12, height / 2 + 3);
        } else {
          const peaks = track.overviewPeaks || [];
          const barWidth = width / peaks.length;
          const halfHeight = height / 2;
          bCtx.fillStyle = isActive ? "#a8c3d8" : "#556673";
          for (let i = 0; i < peaks.length; i += 2) {
            const val = peaks[i];
            const barHeight = val * (height * 0.7);
            const x = i * barWidth;
            const y = halfHeight - barHeight / 2;
            bCtx.fillRect(x, y, Math.max(1, barWidth * 2 - 0.5), barHeight);
          }

          // Draw Markers on Overview Waveform
          if (track.duration > 0 && markers.length > 0) {
            for (const marker of markers) {
              const markerPct = marker.time / track.duration;
              if (markerPct >= 0 && markerPct <= 1.0) {
                const markerX = Math.round(markerPct * width);
                const color = marker.color || "#ff9500";
                bCtx.strokeStyle = color;
                bCtx.lineWidth = 1.0;
                bCtx.beginPath();
                bCtx.moveTo(markerX + 0.5, 0);
                bCtx.lineTo(markerX + 0.5, height);
                bCtx.stroke();

                // Top flag cap
                bCtx.fillStyle = color;
                bCtx.fillRect(markerX - 1.5, 0, 3, 4);
              }
            }
          }

          // Draw Regions on Overview Waveform
          if (track.duration > 0 && regions.length > 0) {
            for (const reg of regions) {
              const regStartPct = reg.startTime / track.duration;
              const regEndPct = reg.endTime / track.duration;
              const rx1 = Math.round(regStartPct * width);
              const rx2 = Math.round(regEndPct * width);
              const rw = Math.max(1, rx2 - rx1);

              if (reg.isCut) {
                // Cut region background & solid vertical lines on overview (1px)
                bCtx.fillStyle = "rgba(255, 69, 58, 0.22)";
                bCtx.fillRect(rx1, 0, rw, height);
                bCtx.strokeStyle = "#ff453a";
                bCtx.lineWidth = 1.0;
                bCtx.beginPath();
                bCtx.moveTo(rx1, 0);
                bCtx.lineTo(rx1, height);
                bCtx.moveTo(rx2, 0);
                bCtx.lineTo(rx2, height);
                bCtx.stroke();
              } else if (reg.isLoop) {
                // Loop region background & solid vertical lines on overview (1px)
                bCtx.fillStyle = "rgba(48, 209, 88, 0.18)";
                bCtx.fillRect(rx1, 0, rw, height);
                bCtx.strokeStyle = "#30d158";
                bCtx.lineWidth = 1.0;
                bCtx.beginPath();
                bCtx.moveTo(rx1, 0);
                bCtx.lineTo(rx1, height);
                bCtx.moveTo(rx2, 0);
                bCtx.lineTo(rx2, height);
                bCtx.stroke();
              }
            }
          }
        }
        bCtx.restore();
        isOverviewBaseDirty = false;
      }
    }

    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.save();
    ctx.scale(dpr, dpr);

    // Blit cached background peaks and static markers (0.005ms GPU texture blit)
    ctx.drawImage(overviewBaseCanvas, 0, 0, width, height);

    // Draw dynamic interactive layer (zoom window viewport and moving playhead)
    const isActive = activeTrackMode === mode;
    if (isActive && track) {
      const windowWidth = 1.0 / zoom;
      const startProgress = Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2));
      const endProgress = startProgress + windowWidth;

      if (zoom > 1.001) {
        ctx.fillStyle = "rgba(59, 153, 252, 0.18)"; 
        ctx.fillRect(startProgress * width, 0, (endProgress - startProgress) * width, height);
        
        ctx.strokeStyle = "#3b99fc";
        ctx.lineWidth = 1;
        ctx.strokeRect(startProgress * width, 0, (endProgress - startProgress) * width, height);
      }

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
    const currentTrack = activeTrackMode === "main" ? mainTrack : alternateTrack;
    drawGenericOverview(overviewCanvas, currentTrack, activeTrackMode);
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

    if (activeKnob.id === "speed") {
      invoke("set_speed", { speed: newVal });
    } else if (activeKnob.id === "pitch" || activeKnob.id === "pitch_cents") {
      updatePitchEngine();
    }
  }

  function handleKnobMouseup() {
    if (activeKnob) {
      if (activeKnob.id === "speed") {
        invoke("set_speed", { speed });
      } else if (activeKnob.id === "pitch" || activeKnob.id === "pitch_cents") {
        updatePitchEngine();
      }
      saveCurrentTrackProfile(filePath);
    }
    activeKnob = null;
    window.removeEventListener("mousemove", handleKnobMousemove);
    window.removeEventListener("mouseup", handleKnobMouseup);
  }

  function resetKnob(id: string, defaultVal: number, setValue: (v: number) => void) {
    setValue(defaultVal);
    if (id === "speed") {
      invoke("set_speed", { speed: defaultVal });
    } else if (id === "pitch" || id === "pitch_cents") {
      updatePitchEngine();
    }
    saveCurrentTrackProfile(filePath);
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

<main class="app-container">
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
                class:highlighted={selectedPlaylistIndex === idx}
                on:click={() => { selectedPlaylistIndex = idx; }}
                on:dblclick={() => {
                  selectedPlaylistIndex = idx;
                  loadAudioPath(item.path, "main", true).then(async () => {
                    await invoke("play");
                    isPlaying = true;
                  });
                }}
              >
                <span class="item-icon">🎵</span>
                <span class="item-name" title={item.name}>{item.name}</span>
                <button class="remove-playlist-item-btn" on:click|stopPropagation={() => removePlaylistItem(idx)}>×</button>
              </div>
            {/each}
          {/if}
        </div>

        <div class="playlist-controls-sidebar">
          <button class="action-btn file-btn" on:click={selectPlaylistFiles} title="Add audio files to playlist">
            + Add
          </button>
          <button class="action-btn save-set-btn" on:click={savePlaylistToFile} title="Save playlist (.thset / .m3u8)">
            💾 Save
          </button>
          <button class="action-btn open-set-btn" on:click={() => loadPlaylistFromFile()} title="Open playlist (.thset / .m3u8)">
            📂 Open
          </button>
          <button class="action-btn clear-btn" on:click={clearPlaylist} title="Clear playlist">
            Clear
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
      
      <!-- Current File info header (Single compact row) -->
      <div class="track-header">
        <div class="track-title-info">
          <span class="track-badge" class:main-badge={activeTrackMode === "main"}>
            {activeTrackMode.toUpperCase()}
          </span>
          <span class="track-title-text" title={filePath}>
            {fileName || "Drag & Drop Audio file to begin"}
          </span>
        </div>
        <div class="header-export-wrap">
          <button 
            class="export-audio-header-btn" 
            title="Export audio with baked DSP, pitch, speed, and cuts (Cmd+Shift+E)"
            on:click={openExportModal}
            disabled={!filePath || duration === 0}
          >
            💾 Export Audio...
          </button>
        </div>
        <div class="time-readout">
          <span class="time-large">{formatTime(currentTime)}</span>
          <span class="time-sep">/</span>
          <span class="time-total">{formatTime(duration)}</span>
        </div>
      </div>

      <!-- Upper Rehearsal Deck (Tabbed inspection area in remaining space) -->
      <div class="rehearsal-bottom-deck">
        <div class="deck-tabs-header">
          <button 
            class="deck-tab-btn" 
            class:active={activeCenterTab === "notes"} 
            on:click={() => switchCenterTab("notes")}
          >
            NOTES
          </button>
          <button 
            class="deck-tab-btn" 
            class:active={activeCenterTab === "lyrics"} 
            on:click={() => switchCenterTab("lyrics")}
          >
            LYRICS
          </button>
          <button 
            class="deck-tab-btn" 
            class:active={activeCenterTab === "metadata"} 
            on:click={() => switchCenterTab("metadata")}
          >
            METADATA
          </button>
          <button 
            class="deck-tab-btn" 
            class:active={activeCenterTab === "files"} 
            on:click={() => switchCenterTab("files")}
          >
            FILES {#if associatedFiles.length > 0}<span class="files-badge-count">({associatedFiles.length})</span>{/if}
          </button>

          <!-- Dynamic Open PDF Tabs -->
          {#each openPdfTabs as tab (tab.id)}
            <button 
              class="deck-tab-btn pdf-tab-btn" 
              class:active={activeCenterTab === "pdf-" + tab.id} 
              on:click={() => switchCenterTab("pdf-" + tab.id)}
              title={tab.path}
            >
              📄 {tab.name}
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <span 
                class="tab-close-btn" 
                title="Close sheet music tab"
                on:click|stopPropagation={() => closePdfTab(tab.id)}
              >
                ×
              </span>
            </button>
          {/each}
        </div>

        <div class="deck-tab-content">
          {#if activeCenterTab === "notes"}
            <!-- NOTES TAB (Markdown Compatible) -->
            <div class="tab-pane notes-pane markdown-deck-pane">
              <div class="markdown-toolbar">
                <span class="md-toolbar-title">REHEARSAL NOTES</span>
                <div class="md-view-toggles">
                  <button class="md-toggle-btn" class:active={notesViewMode === 'edit'} on:click={() => notesViewMode = 'edit'}>✏️ Edit</button>
                  <button class="md-toggle-btn" class:active={notesViewMode === 'preview'} on:click={() => notesViewMode = 'preview'}>👁️ Preview</button>
                  <button class="md-toggle-btn" class:active={notesViewMode === 'split'} on:click={() => notesViewMode = 'split'}>◫ Split</button>
                </div>
              </div>

              <div class="markdown-editor-container mode-{notesViewMode}">
                {#if notesViewMode === 'edit' || notesViewMode === 'split'}
                  <textarea 
                    class="rehearsal-textarea md-textarea" 
                    placeholder="Type rehearsal notes, chord cues (e.g. [C#m7]), key changes, or arrangement details using Markdown (auto-saved)..."
                    bind:value={songNotes}
                    on:input={() => saveCurrentTrackProfile(filePath)}
                  ></textarea>
                {/if}

                {#if notesViewMode === 'preview' || notesViewMode === 'split'}
                  <div class="markdown-preview-pane">
                    {@html renderMarkdown(songNotes)}
                  </div>
                {/if}
              </div>
            </div>
          {:else if activeCenterTab === "lyrics"}
            <!-- LYRICS TAB (Markdown Compatible) -->
            <div class="tab-pane lyrics-pane markdown-deck-pane">
              <div class="markdown-toolbar">
                <span class="md-toolbar-title">SONG LYRICS & CUES</span>
                <div class="md-view-toggles">
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'edit'} on:click={() => lyricsViewMode = 'edit'}>✏️ Edit</button>
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'preview'} on:click={() => lyricsViewMode = 'preview'}>👁️ Preview</button>
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'split'} on:click={() => lyricsViewMode = 'split'}>◫ Split</button>
                </div>
              </div>

              <div class="markdown-editor-container mode-{lyricsViewMode}">
                {#if lyricsViewMode === 'edit' || lyricsViewMode === 'split'}
                  <textarea 
                    class="rehearsal-textarea md-textarea lyrics-textarea" 
                    placeholder="Type or paste lyrics with chord badges e.g. [G/B] or [Am7] (auto-saved with this song)..."
                    bind:value={songLyrics}
                    on:input={() => saveCurrentTrackProfile(filePath)}
                  ></textarea>
                {/if}

                {#if lyricsViewMode === 'preview' || lyricsViewMode === 'split'}
                  <div class="markdown-preview-pane lyrics-preview">
                    {@html renderMarkdown(songLyrics)}
                  </div>
                {/if}
              </div>
            </div>
          {:else if activeCenterTab === "files"}
            <!-- FILES TAB (Associated Media, PDFs, Stems, Alternate Versions) -->
            <div class="tab-pane files-pane">
              <div class="files-pane-header">
                <div class="files-header-left">
                  <span class="files-pane-title">ASSOCIATED MEDIA & FILES</span>
                  <span class="files-pane-subtitle">PDF sheet music, chord charts, backing tracks, stems, and alternate rehearsal takes</span>
                </div>
                <button class="add-assoc-file-btn" on:click={addAssociatedFilePicker}>
                  + Add Associated File...
                </button>
              </div>

              <div class="associated-files-list">
                {#if associatedFiles.length === 0}
                  <div class="empty-files-card">
                    <span class="empty-icon">📁</span>
                    <h3>No Associated Files Linked Yet</h3>
                    <p>Link PDF lead sheets, score charts, backing tracks, or stems to this song for quick access during rehearsal.</p>
                    <div class="empty-actions-row">
                      <button class="action-card-btn" on:click={addAssociatedFilePicker}>
                        📄 Link Sheet Music (PDF)
                      </button>
                      <button class="action-card-btn" on:click={addAssociatedFilePicker}>
                        🎵 Link Alternate Track / Stem
                      </button>
                    </div>
                  </div>
                {:else}
                  <div class="assoc-files-grid">
                    {#each associatedFiles as item (item.id)}
                      <div class="assoc-card" class:pdf-card={item.fileType === 'pdf'} class:audio-card={item.fileType === 'audio'}>
                        <div class="assoc-card-icon">
                          {item.fileType === 'pdf' ? '📄' : '🎵'}
                        </div>
                        <div class="assoc-card-info">
                          <span class="assoc-card-name" title={item.name}>{item.name}</span>
                          <span class="assoc-card-path" title={item.path}>{item.path}</span>
                        </div>
                        <div class="assoc-card-actions">
                          {#if item.fileType === 'pdf'}
                            <button 
                              class="assoc-action-btn open-pdf-action" 
                              on:click={() => openPdfTab(item.path, item.name)}
                              title="Open PDF Sheet Music in Tab"
                            >
                              📄 Open Tab
                            </button>
                          {:else if item.fileType === 'audio'}
                            <button 
                              class="assoc-action-btn load-main-action" 
                              on:click={() => loadAudioPath(item.path, 'main')}
                              title="Load as Main Track"
                            >
                              ▶ Load Main
                            </button>
                            <button 
                              class="assoc-action-btn load-alt-action" 
                              on:click={() => loadAudioPath(item.path, 'alternate')}
                              title="Load as Alternate Track"
                            >
                              ⌥ Load Alt
                            </button>
                          {/if}
                          <button 
                            class="assoc-action-btn unlink-action" 
                            on:click={() => unlinkAssociatedFile(item.id)}
                            title="Unlink file from this project"
                          >
                            ×
                          </button>
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {:else if activeCenterTab === "metadata"}
            <!-- METADATA TAB -->
            <div class="tab-pane metadata-pane">
              <div class="metadata-split-layout">
                <!-- Left: Editable Audio Tags (Lofty ID3 / Vorbis / MP4 / FLAC) -->
                <div class="metadata-edit-section">
                  <div class="meta-section-header">
                    <span class="meta-section-title">AUDIO TAGS (EDITABLE)</span>
                    <div class="meta-save-row">
                      {#if tagSaveFeedback}
                        <span class="tag-save-feedback">{tagSaveFeedback}</span>
                      {/if}
                      <button 
                        class="save-tags-btn" 
                        on:click={saveAudioTags}
                        disabled={isSavingTags || !filePath}
                      >
                        {isSavingTags ? "Saving..." : "💾 Save Tags to Audio File"}
                      </button>
                    </div>
                  </div>

                  <div class="meta-form-grid">
                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-title">Title</label>
                      <input 
                        id="tag-title"
                        type="text" 
                        class="meta-input" 
                        placeholder="Song Title" 
                        bind:value={audioTags.title} 
                      />
                    </div>

                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-artist">Artist</label>
                      <input 
                        id="tag-artist"
                        type="text" 
                        class="meta-input" 
                        placeholder="Artist / Performer" 
                        bind:value={audioTags.artist} 
                      />
                    </div>

                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-album">Album</label>
                      <input 
                        id="tag-album"
                        type="text" 
                        class="meta-input" 
                        placeholder="Album / Project" 
                        bind:value={audioTags.album} 
                      />
                    </div>

                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-grouping">Grouping / Movement</label>
                      <input 
                        id="tag-grouping"
                        type="text" 
                        class="meta-input" 
                        placeholder="Grouping / Scene / Act / Band" 
                        bind:value={audioTags.grouping} 
                      />
                    </div>

                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-composer">Composer</label>
                      <input 
                        id="tag-composer"
                        type="text" 
                        class="meta-input" 
                        placeholder="Composer / Arranger" 
                        bind:value={audioTags.composer} 
                      />
                    </div>

                    <div class="meta-field-group">
                      <label class="meta-field-label" for="tag-genre">Genre</label>
                      <input 
                        id="tag-genre"
                        type="text" 
                        class="meta-input" 
                        placeholder="Genre (e.g. Jazz, Rock, Classical)" 
                        bind:value={audioTags.genre} 
                      />
                    </div>

                    <div class="meta-field-row">
                      <div class="meta-field-group half">
                        <label class="meta-field-label" for="tag-year">Year</label>
                        <input 
                          id="tag-year"
                          type="number" 
                          class="meta-input" 
                          placeholder="YYYY" 
                          bind:value={audioTags.year} 
                        />
                      </div>
                      <div class="meta-field-group half">
                        <label class="meta-field-label" for="tag-track">Track #</label>
                        <input 
                          id="tag-track"
                          type="number" 
                          class="meta-input" 
                          placeholder="No." 
                          bind:value={audioTags.track_number} 
                        />
                      </div>
                    </div>

                    <div class="meta-field-group full">
                      <label class="meta-field-label" for="tag-comment">Comment / Notes</label>
                      <input 
                        id="tag-comment"
                        type="text" 
                        class="meta-input" 
                        placeholder="Audio tag comment" 
                        bind:value={audioTags.comment} 
                      />
                    </div>
                  </div>
                </div>

                <!-- Right: Audio File Specs -->
                <div class="metadata-specs-section">
                  <span class="meta-section-title">FILE PROPERTIES</span>
                  <div class="specs-grid">
                    <div class="spec-row">
                      <span class="spec-label">File:</span>
                      <span class="spec-val" title={fileName}>{fileName || "None"}</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Path:</span>
                      <span class="spec-val path-val" title={filePath}>{filePath || "None"}</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Duration:</span>
                      <span class="spec-val">{formatTime(duration)} ({duration.toFixed(2)}s)</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Sample Rate:</span>
                      <span class="spec-val">{sampleRate} Hz ({sampleRate / 1000} kHz)</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Channels:</span>
                      <span class="spec-val">{channels === 2 ? "Stereo (2 ch)" : channels === 1 ? "Mono (1 ch)" : `${channels} ch`}</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Active Slot:</span>
                      <span class="spec-val active-slot">{activeTrackMode.toUpperCase()}</span>
                    </div>
                    <div class="spec-row">
                      <span class="spec-label">Markers:</span>
                      <span class="spec-val">{markers.length} markers</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          {:else if activeCenterTab.startsWith("pdf-")}
            <!-- DYNAMIC MULTI-PDF VIEWER TAB -->
            {@const currentTab = openPdfTabs.find(t => t.id === activePdfTabId)}
            {#if currentTab}
              <div class="tab-pane pdf-pane">
                <div class="pdf-viewer-container">
                  <div class="pdf-floating-controls">
                    {#if currentTab.totalPages > 0}
                      <span class="pdf-page-pill">{currentTab.currentPage}/{currentTab.totalPages}</span>
                    {/if}
                    <button 
                      class="pdf-mini-btn" 
                      class:active-toggle={currentTab.isInverted}
                      on:click={() => toggleDynamicTabInvert(currentTab.id)} 
                      title="Toggle Inverted / Negative Dark Mode"
                    >
                      {currentTab.isInverted ? "☀️ Normal" : "🌙 Invert"}
                    </button>
                    <button class="pdf-mini-btn popout" on:click={() => invoke("open_file_external", { path: currentTab.path })} title="Open in System Default Viewer">
                      ⤢ External
                    </button>
                    <button class="pdf-mini-btn close-btn" on:click={() => closePdfTab(currentTab.id)} title="Close tab">
                      × Close
                    </button>
                  </div>

                  {#if currentTab.isLoading}
                    <div class="pdf-loading-overlay">
                      <span class="pdf-loading-spinner">⏳</span>
                      <span>Rendering sheet music...</span>
                    </div>
                  {/if}

                  {#if currentTab.error}
                    <div class="pdf-error-card">
                      <span>⚠️ {currentTab.error}</span>
                      <button class="retry-pdf-btn" on:click={() => renderOpenPdfTab(currentTab)}>Retry</button>
                    </div>
                  {/if}

                  <div 
                    class="pdf-scroll-column" 
                    id={"pdf-container-" + currentTab.id}
                    on:scroll={(e) => handleDynamicPdfScroll(e, currentTab)}
                    on:dragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; }}
                    on:drop={(e) => handleDynamicPdfContainerDrop(e, currentTab)}
                  ></div>
                </div>
              </div>
            {/if}
          {/if}
        </div>
      </div>

      <!-- Waveforms (Single Overview 40px, Main Waveform 256px) -->
      <div class="waveforms-flexbox">
        <!-- Single Dynamic Overview Waveform (40px) -->
        <div class="waveform-block block-overview" title="Right-click for Main / Alternate track options">
          <span 
            class="overview-watermark-tag" 
            class:alt-tag={activeTrackMode === "alternate"} 
            class:main-tag={activeTrackMode === "main"}
          >
            {activeTrackMode.toUpperCase()}
          </span>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={overviewCanvas} 
            on:mousedown={(e) => handleOverviewMouseDown(e, activeTrackMode)}
            on:contextmenu={handleWaveformContextMenu}
            class="overview-canvas"
          ></canvas>
        </div>

        <!-- Main Waveform Box (256px height) -->
        <div 
          class="waveform-block block-main-waveform" 
          title="Right-click for Main / Alternate track options • Drop marker to place"
          on:dragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; }}
          on:drop={handleWaveformMarkerDrop}
        >
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={mainCanvas} 
            on:mousedown={handleMainMouseDown}
            on:mousemove={handleMainCanvasHover}
            on:wheel|passive={handleMainWheel}
            on:contextmenu={handleWaveformContextMenu}
            class="main-canvas"
          ></canvas>
        </div>
      </div>

      <!-- Bottom controls bar (Transport & Loop controls) -->
      <div class="controls-row">
        <!-- Transport controls -->
        <div class="control-group transport-group">
          <div class="btn-row">
            <button class="control-btn" on:click={handleRewind} title="Rewind to start">⏮</button>
            <button class="control-btn" on:click={jumpToPrevMarker} title="Previous Marker (←)">⏪</button>
            <button class="control-btn play-btn" on:click={handlePlayPause} title="Play / Pause (Space)">
              {isPlaying ? "⏸ PAUSE" : "▶ PLAY"}
            </button>
            <button class="control-btn stop-btn" on:click={handleStop} title="Stop All Tracks (ESC / Enter)">
              ⏹ STOP
            </button>
            <button class="control-btn" on:click={jumpToNextMarker} title="Next Marker (→)">⏩</button>
          </div>
        </div>

        <!-- Looping, markers, and Zoom controls -->
        <div class="control-group loop-group">
          <div class="loop-zoom-grid">
            <div class="btn-row">
              <button class="control-btn accent-btn" on:click={addMarker} title="Add Landmark / Marker at playhead (M)">+ Marker</button>
              <button class="control-btn" on:click={() => createRegionFromSelectionOrMarkers(false, false)} title="Create Region from active selection or markers (R)">+ Region</button>
              <button class="control-btn" on:click={handleLoopHotkey} title="Toggle Loop mode on selected region or create loop (L)">🔁 Loop</button>
            </div>
            <div class="zoom-slider-group">
              <span class="control-text-label">ZOOM</span>
              <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
              <div class="zoom-track-wrapper">
                <input 
                  type="range" 
                  min="0" 
                  max="1000" 
                  step="1" 
                  bind:value={zoomSliderVal} 
                  on:dblclick={() => { setZoom(target15sZoom); updateVisiblePeaks(); }}
                  on:input={handleZoomSliderInput}
                  class="zoom-slider" 
                />
                {#if duration > 15}
                  <div 
                    class="zoom-snap-notch" 
                    style="left: {(zoomToSliderVal(target15sZoom) / 1000) * 100}%;" 
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

    <!-- RIGHT SIDEBAR: Markers & Regions, Effects Rack (Dorico Theme) -->
    <aside class="sidebar-right">
      
      <!-- Markers & Regions List (Dedicated full vertical height) -->
      <div class="panel-section markers-section">
        <div class="panel-header">
          MARKERS & REGIONS
          <span class="marker-track-mode-badge" class:alt-badge={activeTrackMode === 'alternate'}>
            ({activeTrackMode.toUpperCase()})
          </span>
        </div>
        <div class="markers-list">
          {#if markers.length === 0}
            <div class="placeholder-text">
              {#if activeTrackMode === 'alternate'}
                No markers set for Alternate Track. Drag from below or click "+ Add Marker".
              {:else}
                No markers set. Click "+ Add Marker" during playback.
              {/if}
            </div>
          {:else}
            {#each markers as marker}
              <div 
                class="marker-item" 
                class:selected-marker-item={selectedMarkerIds.has(marker.id)}
                style="border-left: 3px solid {marker.color || '#ff9500'};"
                on:mousedown={(e) => handleMarkerItemMouseDown(e, marker)}
                title="Click to seek • Shift-click to select pair • Double-click to rename • Drag to waveform or sheet music"
              >
                <!-- Color Dot Palette Opener -->
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <!-- svelte-ignore a11y-no-static-element-interactions -->
                <span 
                  class="marker-color-dot" 
                  style="background-color: {marker.color || '#ff9500'};"
                  title="Click to choose color"
                  on:click={(e) => openColorPalette(e, marker)}
                ></span>

                {#if editingMarkerId === marker.id}
                  <input
                    type="text"
                    class="marker-rename-input"
                    bind:value={editingMarkerName}
                    on:keydown={(e) => {
                      if (e.key === "Enter") saveRenameMarker(marker);
                      if (e.key === "Escape") cancelRenameMarker();
                    }}
                    on:blur={() => saveRenameMarker(marker)}
                    autofocus
                  />
                {:else}
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <!-- svelte-ignore a11y-no-static-element-interactions -->
                  <span 
                    class="marker-name-btn" 
                    on:click={(e) => {
                      if (e.shiftKey) {
                        handleMarkerItemMouseDown(e, marker);
                      } else {
                        seekToMarker(marker.time);
                      }
                    }}
                    on:dblclick={() => startRenameMarker(marker)}
                    title="Click to seek • Shift-click to select pair • Double-click to rename • Drag to waveform or PDF"
                  >
                    {marker.name} <span class="marker-time-tag">({formatTime(marker.time)})</span>
                  </span>
                {/if}

                {#if marker.pdfAnchor}
                  <span class="marker-pdf-tag" title="Pinned to Sheet Music (Page {marker.pdfAnchor.page})">📄 p.{marker.pdfAnchor.page}</span>
                {/if}

                <div class="marker-item-actions">
                  <button 
                    class="marker-action-btn" 
                    title="Rename marker" 
                    on:click={() => startRenameMarker(marker)}
                  >
                    ✏️
                  </button>
                  <button 
                    class="delete-marker-btn" 
                    title="Delete marker"
                    on:click={() => deleteMarker(marker.id)}
                  >
                    ×
                  </button>
                </div>
              </div>
            {/each}
          {/if}

          <!-- Shared Project Landmarks Bin -->
          {#if projectUnplacedLandmarks.length > 0}
            <div class="unplaced-markers-header">
              <span>UNASSIGNED</span>
              <span class="unplaced-hint">Drag to place</span>
            </div>
            {#each projectUnplacedLandmarks as landmark}
              <div 
                class="marker-item unplaced-main-marker"
                style="border-left: 3px dashed {landmark.color || '#ff9500'};"
                on:mousedown={(e) => handleMarkerItemMouseDown(e, landmark)}
                title="Drag onto active waveform ({activeTrackMode.toUpperCase()}) or score to place"
              >
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <!-- svelte-ignore a11y-no-static-element-interactions -->
                <span 
                  class="marker-color-dot" 
                  style="background-color: {landmark.color || '#ff9500'};"
                  title="Click to choose color"
                  on:click={(e) => openColorPalette(e, landmark)}
                ></span>
                {#if editingMarkerId === landmark.id}
                  <input
                    type="text"
                    class="marker-rename-input"
                    bind:value={editingMarkerName}
                    on:keydown={(e) => {
                      if (e.key === "Enter") saveRenameMarker(landmark);
                      if (e.key === "Escape") cancelRenameMarker();
                    }}
                    on:blur={() => saveRenameMarker(landmark)}
                    autofocus
                  />
                {:else}
                  <span 
                    class="marker-name-btn unplaced-name"
                    on:dblclick={() => startRenameMarker(landmark)}
                    title="Double-click to rename • Drag to waveform or score"
                  >
                    {landmark.name}
                  </span>
                {/if}
                <span class="marker-drag-hint">⇄ Drag to place</span>
                <div class="marker-item-actions">
                  <button 
                    class="marker-action-btn" 
                    title="Rename landmark" 
                    on:click|stopPropagation={() => startRenameMarker(landmark)}
                  >
                    ✏️
                  </button>
                  <button 
                    class="delete-marker-btn" 
                    title="Delete landmark from project"
                    on:click|stopPropagation={() => deleteProjectLandmark(landmark.name)}
                  >
                    ×
                  </button>
                </div>
              </div>
            {/each}
          {/if}

          <!-- Regions List -->
          {#if regions.length > 0}
            <div class="regions-header-row">
              <span class="regions-header-title">REGIONS</span>
              <span class="regions-count">{regions.length}</span>
            </div>
            {#each regions as region}
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div 
                class="region-sidebar-item" 
                class:active-region={selectedRegionId === region.id}
                class:is-loop={region.isLoop}
                class:is-cut={region.isCut}
                on:click={() => { selectedRegionId = region.id; }}
                on:contextmenu={(e) => openRegionContextMenu(e, region)}
              >
                <div class="region-item-main">
                  <span class="region-color-bar" style="background-color: {region.isCut ? '#ff453a' : region.isLoop ? '#30d158' : '#0a84ff'}"></span>
                  <div class="region-item-info">
                    {#if editingRegionId === region.id}
                      <input 
                        type="text" 
                        class="marker-rename-input"
                        bind:value={editingRegionName} 
                        on:keydown={(e) => { if (e.key === "Enter") saveRenameRegion(region); if (e.key === "Escape") editingRegionId = null; }}
                        on:blur={() => saveRenameRegion(region)}
                        autofocus
                      />
                    {:else}
                      <span class="region-name" on:dblclick={() => startRenameRegion(region)} title="Double-click to rename">{region.name}</span>
                    {/if}
                    <span class="region-span">{formatTime(region.startTime)} – {formatTime(region.endTime)}</span>
                  </div>
                </div>

                <div class="region-item-toggles">
                  {#if region.isCut}
                    <button 
                      class="region-xfade-pill" 
                      title="Click to edit cut splice crossfade (ms)"
                      on:click|stopPropagation={() => {
                        const val = prompt("Enter cut splice crossfade (0 - 100 ms):", (region.crossfadeMs ?? 5).toString());
                        if (val !== null) {
                          const num = parseFloat(val);
                          if (!isNaN(num)) {
                            region.crossfadeMs = Math.max(0, Math.min(100, num));
                            syncRegionsToEngine();
                          }
                        }
                      }}
                    >
                      ⚡ {region.crossfadeMs ?? 5}ms
                    </button>
                  {/if}
                  <button 
                    class="marker-action-btn" 
                    title="Rename region" 
                    on:click|stopPropagation={() => startRenameRegion(region)}
                  >
                    ✏️
                  </button>
                  <button 
                    class="region-toggle-btn" 
                    class:active={region.isLoop} 
                    title="Toggle Loop / Vamp mode (L)"
                    on:click|stopPropagation={() => toggleRegionLoop(region)}
                  >
                    🔁
                  </button>
                  <button 
                    class="region-toggle-btn cut-toggle-btn" 
                    class:active={region.isCut} 
                    title="Toggle Cut / Skip mode (X)"
                    on:click|stopPropagation={() => toggleRegionCut(region)}
                  >
                    ✂️
                  </button>
                  <button 
                    class="delete-marker-btn" 
                    title="Delete region"
                    on:click|stopPropagation={() => deleteRegion(region.id)}
                  >
                    ×
                  </button>
                </div>
              </div>
            {/each}
          {/if}
        </div>
      </div>

      <!-- DSP Effects Rack Panel -->
      <div class="panel-section dsp-section">
        <div class="panel-header">EFFECTS & DSP</div>

        <!-- 4. Compressor row with 90° COMP Tab Button & BYP toggle -->
        <div class="effects-module-row">
          <div class="module-tab-col">
            <button 
              class="module-tab-btn comp-btn" 
              on:click={() => showAdvancedCompModal = true}
              title="Open Advanced Dynamic Compressor Inspector"
            >
              <span>COMP</span>
            </button>
            <button 
              class="module-bypass-btn" 
              class:is-bypassed={isCompressorBypassed}
              on:click={() => { isCompressorBypassed = !isCompressorBypassed; updateCompressorEngine(); }}
              title="Toggle Compressor Bypass"
            >
              {isCompressorBypassed ? "BYP" : "ON"}
            </button>
          </div>
          <div class="knobs-row" class:effect-bypassed={isCompressorBypassed}>
            <!-- Threshold (0 dB down to -60 dB, default 0 dB) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "comp_thresh", compStage1.thresholdDb, -60, 0, 1, (v) => { compStage1.thresholdDb = v; updateCompressorEngine(); })}
              on:dblclick={() => resetKnob("comp_thresh", 0, (v) => { compStage1.thresholdDb = v; updateCompressorEngine(); })}
              title="Threshold (0 dB to -60 dB) • Double-click resets to 0 dB"
            >
              <span class="knob-label">Threshold</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(compStage1.thresholdDb, -60, 0)}deg)"></div>
              </div>
              <span class="knob-value">{compStage1.thresholdDb.toFixed(0)} dB</span>
            </div>

            <!-- Ratio (1.0:1 up to 4.0:1, default 1.0:1) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "comp_ratio", compStage1.ratio, 1.0, 4.0, 0.1, (v) => { compStage1.ratio = v; updateCompressorEngine(); })}
              on:dblclick={() => resetKnob("comp_ratio", 1.0, (v) => { compStage1.ratio = v; updateCompressorEngine(); })}
              title="Ratio (1:1 to 4:1) • Double-click resets to 1.0:1"
            >
              <span class="knob-label">Ratio</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(compStage1.ratio, 1.0, 4.0)}deg)"></div>
              </div>
              <span class="knob-value">{compStage1.ratio.toFixed(1)}:1</span>
            </div>

            <!-- Makeup (0 dB to 24 dB, default 0 dB) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "comp_makeup", compStage1.makeupDb, 0, 24, 0.5, (v) => { compStage1.makeupDb = v; updateCompressorEngine(); })}
              on:dblclick={() => resetKnob("comp_makeup", 0, (v) => { compStage1.makeupDb = v; updateCompressorEngine(); })}
              title="Makeup Gain • Double-click resets to 0 dB"
            >
              <span class="knob-label">Makeup</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(compStage1.makeupDb, 0, 24)}deg)"></div>
              </div>
              <span class="knob-value">+{compStage1.makeupDb.toFixed(1)} dB</span>
            </div>
          </div>
        </div>

        <!-- 3. Equalizer row with 90° EQ Tab Button & BYP toggle -->
        <div class="effects-module-row">
          <div class="module-tab-col">
            <button 
              class="module-tab-btn eq-btn" 
              on:click={() => showAdvancedEqModal = true}
              title="Open Advanced Parametric EQ Inspector"
            >
              <span>EQ</span>
            </button>
            <button 
              class="module-bypass-btn" 
              class:is-bypassed={isEqBypassed}
              on:click={() => { isEqBypassed = !isEqBypassed; updateEqEngine(); }}
              title="Toggle Equalizer Bypass"
            >
              {isEqBypassed ? "BYP" : "ON"}
            </button>
          </div>
          <div class="knobs-row" class:effect-bypassed={isEqBypassed}>
            <!-- Low Shelf (100 Hz) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "eq_bass", eqBass, -12, 12, 0.5, (v) => { eqBass = v; const n = eqNodes.find(x => x.filterType === 'LowShelf'); if (n) n.gainDb = v; updateEqEngine(); })}
              on:dblclick={() => resetKnob("eq_bass", 0, (v) => { eqBass = v; const n = eqNodes.find(x => x.filterType === 'LowShelf'); if (n) n.gainDb = v; updateEqEngine(); })}
              title="Low Shelf 100 Hz • Double-click resets to 0 dB"
            >
              <span class="knob-label">Low 100Hz</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(eqBass, -12, 12)}deg)"></div>
              </div>
              <span class="knob-value">{eqBass > 0 ? "+" : ""}{eqBass.toFixed(1)} dB</span>
            </div>

            <!-- Mid Parametric Bell (1 kHz) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "eq_mid", eqMid, -12, 12, 0.5, (v) => { eqMid = v; const n = eqNodes.find(x => x.filterType === 'Peaking'); if (n) n.gainDb = v; updateEqEngine(); })}
              on:dblclick={() => resetKnob("eq_mid", 0, (v) => { eqMid = v; const n = eqNodes.find(x => x.filterType === 'Peaking'); if (n) n.gainDb = v; updateEqEngine(); })}
              title="Mid Bell 1 kHz • Double-click resets to 0 dB"
            >
              <span class="knob-label">Mid 1kHz</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(eqMid, -12, 12)}deg)"></div>
              </div>
              <span class="knob-value">{eqMid > 0 ? "+" : ""}{eqMid.toFixed(1)} dB</span>
            </div>

            <!-- High Shelf (8 kHz) -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "eq_treble", eqTreble, -12, 12, 0.5, (v) => { eqTreble = v; const n = eqNodes.find(x => x.filterType === 'HighShelf'); if (n) n.gainDb = v; updateEqEngine(); })}
              on:dblclick={() => resetKnob("eq_treble", 0, (v) => { eqTreble = v; const n = eqNodes.find(x => x.filterType === 'HighShelf'); if (n) n.gainDb = v; updateEqEngine(); })}
              title="High Shelf 8 kHz • Double-click resets to 0 dB"
            >
              <span class="knob-label">High 8kHz</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(eqTreble, -12, 12)}deg)"></div>
              </div>
              <span class="knob-value">{eqTreble > 0 ? "+" : ""}{eqTreble.toFixed(1)} dB</span>
            </div>
          </div>
        </div>

        <!-- 2. Speed, Pitch, and Fine Tune Knobs (3 knobs on same line) -->
        <div class="effects-module-row pitch-speed-module-row">
          <div class="module-tab-col pitch-tab-col">
            <div class="module-tab-btn pitch-btn" title="Pitch Shifting & Time Stretching">
              <span>TIME</span>
            </div>
            <div class="module-bypass-placeholder"></div>
          </div>
          <div class="knobs-row active-knobs">
            <!-- Speed Knob -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "speed", speed, 0.25, 4.00, 0.05, (v) => speed = v)}
              on:dblclick={() => resetKnob("speed", 1.0, (v) => speed = v)}
              title="Speed Tempo • Double-click resets to 100%"
            >
              <span class="knob-label">Speed</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(speed, 0.25, 4.00)}deg)"></div>
              </div>
              <span class="knob-value">{Math.round(speed * 100)}%</span>
            </div>

            <!-- Pitch Shift (Semitones) Knob in the MIDDLE -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "pitch", pitch, -24, 24, 1, (v) => pitch = v)}
              on:dblclick={() => resetKnob("pitch", 0, (v) => pitch = v)}
              title="Pitch Transposition (Semitones) • Double-click resets to 0 st"
            >
              <span class="knob-label">Pitch</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(pitch, -24, 24)}deg)"></div>
              </div>
              <span class="knob-value">{pitch > 0 ? "+" : ""}{pitch} st</span>
            </div>

            <!-- Fine Tune (Cents) Knob on the RIGHT -->
            <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
            <div 
              class="knob-container" 
              on:mousedown={(e) => handleKnobMousedown(e, "pitch_cents", pitchCents, -100, 100, 1, (v) => pitchCents = v)}
              on:dblclick={() => resetKnob("pitch_cents", 0, (v) => pitchCents = v)}
              title="Fine Tune (Cents) • Double-click resets to 0 cents"
            >
              <span class="knob-label">Fine</span>
              <div class="knob-circle">
                <div class="knob-zero-tick"></div>
                <div class="knob-marker" style="transform: rotate({getKnobRotation(pitchCents, -100, 100)}deg)"></div>
              </div>
              <span class="knob-value">{pitchCents > 0 ? "+" : ""}{pitchCents} ct</span>
            </div>
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
              saveCurrentTrackProfile(filePath);
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
      {#if contextMenuType === "waveform"}
        <!-- Waveform Context Menu: Main vs Alternate Track -->
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item" 
          class:menu-active={activeTrackMode === "main"}
          on:click={() => { toggleActiveTrack("main"); showContextMenu = false; }}
        >
          {activeTrackMode === "main" ? "✓ " : "  "}Set to Main Track {mainTrack ? `(${mainTrack.name})` : "(None Loaded)"}
        </div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item" 
          class:menu-active={activeTrackMode === "alternate"}
          on:click={() => { toggleActiveTrack("alternate"); showContextMenu = false; }}
        >
          {activeTrackMode === "alternate" ? "✓ " : "  "}Set to Alternate Track {alternateTrack ? `(${alternateTrack.name})` : "(Click to Choose...)"}
        </div>
        <div class="menu-divider"></div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div class="menu-item" on:click={() => { pickMainTrack(); showContextMenu = false; }}>
          Choose / Replace Main Track...
        </div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div class="menu-item" on:click={() => { pickAlternateTrack(); showContextMenu = false; }}>
          Choose / Replace Alternate Track...
        </div>
        {#if alternateTrack}
          <div class="menu-divider"></div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div class="menu-item menu-item-danger" on:click={() => { alternateTrack = null; if (activeTrackMode === "alternate") toggleActiveTrack("main"); showContextMenu = false; }}>
            Unload Alternate Track
          </div>
        {/if}
      {:else}
        <!-- Browser / Playlist Context Menu -->
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
        <div class="menu-divider"></div>
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
      {/if}
    </div>
  {/if}

  <!-- Marker Color Palette Popover -->
  {#if colorPaletteMarker}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div 
      class="marker-color-palette-popover" 
      style="top: {colorPaletteY}px; left: {colorPaletteX}px;"
      on:click|stopPropagation
    >
      <div class="palette-header">Marker Color</div>
      <div class="palette-grid">
        {#each MARKER_COLORS as color}
          <button 
            type="button"
            class="palette-swatch" 
            class:selected={colorPaletteMarker.color === color}
            style="background-color: {color};" 
            title="Choose {color}"
            on:click={() => setMarkerColor(colorPaletteMarker, color)}
          >
            {#if colorPaletteMarker.color === color}
              <span class="swatch-check">✓</span>
            {/if}
          </button>
        {/each}
      </div>
    </div>
  {/if}
  <!-- Region Context Menu -->
  {#if showRegionContextMenu && contextMenuRegion}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div 
      class="context-menu" 
      style="top: {regionContextMenuY}px; left: {regionContextMenuX}px;"
      on:click|stopPropagation
    >
      <div class="menu-item font-semibold" style="color: #64d2ff; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 4px; margin-bottom: 4px;">
        REGION: {contextMenuRegion.name}
      </div>
      <div class="menu-item" on:click={() => { if (contextMenuRegion) toggleRegionLoop(contextMenuRegion); showRegionContextMenu = false; }}>
        {contextMenuRegion.isLoop ? "✓ " : "  "}🔁 Loop / Vamp Mode
      </div>
      <div class="menu-item" on:click={() => { if (contextMenuRegion) toggleRegionCut(contextMenuRegion); showRegionContextMenu = false; }}>
        {contextMenuRegion.isCut ? "✓ " : "  "}✂️ Cut / Skip Mode
      </div>
      {#if contextMenuRegion.isCut}
        <div class="menu-item" on:click={() => {
          if (contextMenuRegion) {
            const reg = contextMenuRegion;
            showRegionContextMenu = false;
            const val = prompt("Enter cut splice crossfade (0 - 100 ms):", (reg.crossfadeMs ?? 5).toString());
            if (val !== null) {
              const num = parseFloat(val);
              if (!isNaN(num)) {
                reg.crossfadeMs = Math.max(0, Math.min(100, num));
                syncRegionsToEngine();
              }
            }
          }
        }}>
          ⚡ Splice Crossfade ({contextMenuRegion.crossfadeMs ?? 5}ms)...
        </div>
      {/if}
      <div class="menu-item" on:click={() => { if (contextMenuRegion) startRenameRegion(contextMenuRegion); showRegionContextMenu = false; }}>
        ✏️ Rename Region...
      </div>
      <div class="menu-item delete-item" on:click={() => { if (contextMenuRegion) deleteRegion(contextMenuRegion.id); showRegionContextMenu = false; }}>
        🗑️ Delete Region
      </div>
    </div>
  {/if}

  <!-- Advanced Compressor Inspector Modal (Sonitus Inspired) -->
  {#if showAdvancedCompModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => showAdvancedCompModal = false}>
      <div class="inspector-modal advanced-comp-modal" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge comp-badge">COMP</span>
            <h3>Dynamic Dual-Stage Compressor</h3>
            <span class="stage-subhead">Sonitus Pro Architecture</span>
          </div>

          <div class="modal-header-actions">
            <!-- Stage Tabs (1 vs 2) -->
            <div class="comp-stage-tabs">
              <button 
                class="stage-tab-btn" 
                class:active={activeCompStageTab === 1} 
                on:click={() => activeCompStageTab = 1}
              >
                Stage 1 {compStage1.enabled ? "• ON" : "• OFF"}
              </button>
              <button 
                class="stage-tab-btn" 
                class:active={activeCompStageTab === 2} 
                on:click={() => activeCompStageTab = 2}
              >
                Stage 2 {compStage2.enabled ? "• ON" : "• OFF"}
              </button>
            </div>

            <!-- Routing Selector -->
            <div class="comp-routing-selector">
              <button 
                class="routing-btn" 
                class:active={compRouting === 'Series'} 
                on:click={() => { compRouting = 'Series'; updateCompressorEngine(); }}
                title="Series: Stage 1 feeds into Stage 2"
              >
                Series ➔
              </button>
              <button 
                class="routing-btn" 
                class:active={compRouting === 'Parallel'} 
                on:click={() => { compRouting = 'Parallel'; updateCompressorEngine(); }}
                title="Parallel: Stage 1 and Stage 2 blended together"
              >
                Parallel 🔀
              </button>
            </div>

            <button 
              class="modal-byp-btn" 
              class:is-bypassed={isCompressorBypassed} 
              on:click={() => { isCompressorBypassed = !isCompressorBypassed; updateCompressorEngine(); }}
            >
              {isCompressorBypassed ? "BYPASS ON" : "COMPRESSOR ACTIVE"}
            </button>

            <button class="modal-close-btn" on:click={() => showAdvancedCompModal = false}>×</button>
          </div>
        </div>

        <div class="modal-body comp-inspector-body">
          <!-- Sonitus Graph & Metering Console -->
          <div class="sonitus-console-grid">
            
            <!-- 1. Input Meter & Threshold Vertical Slider -->
            <div class="console-meter-col input-meter-col">
              <span class="meter-label">INPUT</span>
              <div class="meter-slider-combo">
                <!-- Dual Peak Meter L/R -->
                <div class="stereo-peak-track">
                  <div class="peak-channel">
                    <div class="peak-fill" style="height: {Math.min(100, Math.max(0, (liveInputPeakL + 60) * (100 / 60)))}%;"></div>
                  </div>
                  <div class="peak-channel">
                    <div class="peak-fill" style="height: {Math.min(100, Math.max(0, (liveInputPeakR + 60) * (100 / 60)))}%;"></div>
                  </div>
                </div>
                <!-- Vertical Threshold Slider -->
                <div class="vert-slider-wrapper">
                  <input 
                    type="range" 
                    min="-60" 
                    max="0" 
                    step="0.5" 
                    bind:value={curCompStage.thresholdDb} 
                    on:input={() => updateCompressorEngine()}
                    class="vert-slider thresh-slider" 
                    title="Threshold: {curCompStage.thresholdDb.toFixed(1)} dB"
                  />
                </div>
              </div>
              <span class="meter-val-tag">{curCompStage.thresholdDb.toFixed(0)} dB</span>
            </div>

            <!-- 2. Central Transfer Function Graph (Sonitus Curve + Live Tracing Dot) -->
            <div class="sonitus-graph-card">
              <svg viewBox="0 0 300 200" class="sonitus-svg">
                <!-- Grid Lines -->
                {#each [0, 1, 2, 3, 4, 5] as i}
                  <line x1="20" y1="{20 + i * 32}" x2="280" y2="{20 + i * 32}" stroke="rgba(255,255,255,0.08)" stroke-width="1" />
                  <line x1="{20 + i * 52}" y1="20" x2="{20 + i * 52}" y2="180" stroke="rgba(255,255,255,0.08)" stroke-width="1" />
                {/each}

                <!-- 1:1 Faint Diagonal Reference Line -->
                <line x1="20" y1="180" x2="280" y2="20" stroke="rgba(255,255,255,0.2)" stroke-width="1.5" stroke-dasharray="3 3" />

                <!-- Dynamic Transfer Function Path -->
                <path 
                  d={getSonitusCurvePath(curCompStage)} 
                  stroke="#ffcc00" 
                  stroke-width="3" 
                  fill="none" 
                  stroke-linecap="round" 
                />

                <!-- Animated Signal Dot tracing along compression transfer line -->
                <circle 
                  cx={getSonitusSignalDot(curCompStage, liveInputPeakL).x} 
                  cy={getSonitusSignalDot(curCompStage, liveInputPeakL).y} 
                  r="5" 
                  fill="#30d158" 
                  stroke="#ffffff" 
                  stroke-width="1.5" 
                />
              </svg>

              <!-- Graph Scale Labels -->
              <div class="graph-axes-labels">
                <span class="axis-lbl top-left">OUT 0 dB</span>
                <span class="axis-lbl bottom-left">IN -60 dB</span>
                <span class="axis-lbl bottom-right">IN 0 dB</span>
                <span class="axis-lbl curve-type-tag">{curCompStage.compType.toUpperCase()} MODE</span>
              </div>
            </div>

            <!-- 3. Gain Reduction Meter -->
            <div class="console-meter-col gr-meter-col">
              <span class="meter-label">GR</span>
              <div class="meter-slider-combo">
                <div class="gr-peak-track">
                  <div class="gr-fill" style="height: {Math.min(100, liveGainReductionDb * (100 / 30))}%;"></div>
                </div>
              </div>
              <span class="meter-val-tag gr-val">-{liveGainReductionDb.toFixed(1)} dB</span>
            </div>

            <!-- 4. Gain Makeup Vertical Slider -->
            <div class="console-meter-col makeup-meter-col">
              <span class="meter-label">MAKEUP</span>
              <div class="meter-slider-combo">
                <div class="vert-slider-wrapper">
                  <input 
                    type="range" 
                    min="0" 
                    max="24" 
                    step="0.5" 
                    bind:value={curCompStage.makeupDb} 
                    on:input={() => updateCompressorEngine()}
                    class="vert-slider makeup-slider" 
                    title="Makeup Gain: +{curCompStage.makeupDb.toFixed(1)} dB"
                  />
                </div>
              </div>
              <span class="meter-val-tag">+{curCompStage.makeupDb.toFixed(1)} dB</span>
            </div>

            <!-- 5. Output Stereo Meter -->
            <div class="console-meter-col output-meter-col">
              <span class="meter-label">OUTPUT</span>
              <div class="meter-slider-combo">
                <div class="stereo-peak-track">
                  <div class="peak-channel out-peak">
                    <div class="peak-fill" style="height: {Math.min(100, Math.max(0, (liveOutputPeakL + 60) * (100 / 66)))}%;"></div>
                  </div>
                  <div class="peak-channel out-peak">
                    <div class="peak-fill" style="height: {Math.min(100, Math.max(0, (liveOutputPeakR + 60) * (100 / 66)))}%;"></div>
                  </div>
                </div>
              </div>
              <span class="meter-val-tag">{liveOutputPeakL.toFixed(0)} dB</span>
            </div>

          </div>

          <!-- Bottom Advanced Parameter Deck -->
          <div class="sonitus-params-rack">
            <!-- Stage Power -->
            <div class="param-knob-box">
              <span class="param-header-label">STAGE POWER</span>
              <button 
                class="stage-power-toggle" 
                class:active={curCompStage.enabled}
                on:click={() => { curCompStage.enabled = !curCompStage.enabled; updateCompressorEngine(); }}
              >
                {curCompStage.enabled ? "ENABLED" : "BYPASS"}
              </button>
            </div>

            <!-- Compressor Type -->
            <div class="param-knob-box">
              <span class="param-header-label">CHARACTER</span>
              <select 
                class="comp-type-dropdown" 
                bind:value={curCompStage.compType} 
                on:change={() => updateCompressorEngine()}
              >
                <option value="Vintage">Vintage (Warm Tube)</option>
                <option value="Modern">Modern (Clean VCA)</option>
                <option value="FET">FET (Lightning Fast)</option>
                <option value="Opto">Opto (Musical Smooth)</option>
              </select>
            </div>

            <!-- Ratio Slider -->
            <div class="param-knob-box">
              <div class="param-title-val">
                <span>Ratio</span>
                <span class="val-highlight">{curCompStage.ratio.toFixed(1)}:1</span>
              </div>
              <input 
                type="range" 
                min="1.0" 
                max="20.0" 
                step="0.1" 
                bind:value={curCompStage.ratio} 
                on:input={() => updateCompressorEngine()}
                class="rack-h-slider" 
              />
            </div>

            <!-- Knee Slider -->
            <div class="param-knob-box">
              <div class="param-title-val">
                <span>Knee</span>
                <span class="val-highlight">{curCompStage.kneeDb.toFixed(1)} dB</span>
              </div>
              <input 
                type="range" 
                min="0.0" 
                max="12.0" 
                step="0.5" 
                bind:value={curCompStage.kneeDb} 
                on:input={() => updateCompressorEngine()}
                class="rack-h-slider" 
              />
            </div>

            <!-- Attack Slider -->
            <div class="param-knob-box">
              <div class="param-title-val">
                <span>Attack</span>
                <span class="val-highlight">{curCompStage.attackMs.toFixed(1)} ms</span>
              </div>
              <input 
                type="range" 
                min="0.1" 
                max="200.0" 
                step="0.5" 
                bind:value={curCompStage.attackMs} 
                on:input={() => updateCompressorEngine()}
                class="rack-h-slider" 
              />
            </div>

            <!-- Release Slider -->
            <div class="param-knob-box">
              <div class="param-title-val">
                <span>Release</span>
                <span class="val-highlight">{curCompStage.releaseMs.toFixed(0)} ms</span>
              </div>
              <input 
                type="range" 
                min="10.0" 
                max="2000.0" 
                step="10" 
                bind:value={curCompStage.releaseMs} 
                on:input={() => updateCompressorEngine()}
                class="rack-h-slider" 
              />
            </div>

            {#if compRouting === 'Parallel'}
              <!-- Parallel Blend Slider -->
              <div class="param-knob-box blend-box">
                <div class="param-title-val">
                  <span>Parallel Blend</span>
                  <span class="val-highlight">{Math.round(compParallelBlend * 100)}% S2</span>
                </div>
                <input 
                  type="range" 
                  min="0.0" 
                  max="1.0" 
                  step="0.01" 
                  bind:value={compParallelBlend} 
                  on:input={() => updateCompressorEngine()}
                  class="rack-h-slider" 
                />
              </div>
            {/if}
          </div>
        </div>

        <div class="modal-footer">
          <span class="footer-hint">Dynamic real-time series/parallel DSP processing on audio thread</span>
          <button class="modal-action-btn" on:click={() => showAdvancedCompModal = false}>Close</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Advanced Parametric EQ Inspector Modal (Kirchhoff & AnyTune Inspired) -->
  {#if showAdvancedEqModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => showAdvancedEqModal = false}>
      <div class="inspector-modal advanced-eq-modal" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge eq-badge">EQ</span>
            <h3>Parametric Equalizer</h3>
            <span class="stage-subhead">64-bit RBJ Cascaded Biquads</span>
          </div>

          <div class="modal-header-actions">
            <button 
              class="add-eq-band-btn" 
              on:click={() => {
                const newId = "node-" + (eqNodes.length + 1);
                eqNodes = [...eqNodes, {
                  id: newId,
                  name: "Band " + (eqNodes.length + 1),
                  filterType: "Peaking",
                  freq: 2500,
                  gainDb: 0.0,
                  q: 1.0,
                  enabled: true,
                  color: "#ff375f"
                }];
                selectedEqNodeId = newId;
                updateEqEngine();
              }}
            >
              + Add Filter Band
            </button>

            <button 
              class="modal-byp-btn" 
              class:is-bypassed={isEqBypassed} 
              on:click={() => { isEqBypassed = !isEqBypassed; updateEqEngine(); }}
            >
              {isEqBypassed ? "BYPASS ON" : "EQ ACTIVE"}
            </button>

            <button class="modal-close-btn" on:click={() => showAdvancedEqModal = false}>×</button>
          </div>
        </div>

        <div class="modal-body eq-inspector-body">
          <!-- Kirchhoff Interactive EQ Graph Canvas -->
          <div class="kirchhoff-eq-canvas-card">
            <svg viewBox="0 0 600 240" class="kirchhoff-eq-svg">
              <!-- Logarithmic Frequency Grid Lines (20Hz to 20kHz) -->
              <!-- log10(20)=1.301, log10(20000)=4.301. range = 3.0. x = (log10(f) - 1.301) / 3.0 * 560 + 20 -->
              {#each [50, 100, 200, 500, 1000, 2000, 5000, 10000] as f}
                {@const x = 20 + ((Math.log10(f) - 1.30103) / 3.0) * 560}
                <line x1={x} y1="20" x2={x} y2="220" stroke="rgba(255,255,255,0.07)" stroke-width="1" />
              {/each}

              <!-- Gain Grid Lines (+24dB to -24dB) -->
              {#each [-24, -18, -12, -6, 0, 6, 12, 18, 24] as g}
                {@const y = 120 - g * (100 / 24)}
                <line x1="20" y1={y} x2="580" y2={y} stroke={g === 0 ? "rgba(255,255,255,0.25)" : "rgba(255,255,255,0.07)"} stroke-width={g === 0 ? "1.5" : "1"} />
                <text x="24" y={y - 3} fill="rgba(255,255,255,0.3)" font-size="9">{g > 0 ? "+" : ""}{g}dB</text>
              {/each}

              <!-- Ghosted Incoming Spectrum Representation (Simulated Audio Presence) -->
              <path 
                d="M 20 220 Q 80 {isPlaying ? 160 + Math.sin(Date.now()/200)*20 : 210} 180 {isPlaying ? 140 + Math.cos(Date.now()/180)*25 : 210} T 350 {isPlaying ? 150 + Math.sin(Date.now()/160)*18 : 210} T 520 {isPlaying ? 180 + Math.cos(Date.now()/220)*15 : 210} L 580 220 Z" 
                fill="rgba(100, 210, 255, 0.08)" 
                stroke="rgba(100, 210, 255, 0.2)" 
                stroke-width="1" 
              />

              <!-- Cumulative EQ Curve Path -->
              <path 
                d={getEqPath(eqNodes)} 
                stroke="#64d2ff" 
                stroke-width="3" 
                fill="none" 
                stroke-linecap="round" 
              />

              <!-- Interactive Node Handles -->
              {#each eqNodes as node}
                {@const nx = 20 + ((Math.log10(Math.max(20, Math.min(20000, node.freq))) - 1.30103) / 3.0) * 560}
                {@const ny = node.enabled ? 120 - Math.max(-24, Math.min(24, node.gainDb)) * (100 / 24) : 120}
                {@const isSel = selectedEqNodeId === node.id}
                {@const qWidth = Math.max(15, 60 / node.q)}

                <!-- Selected Node Q Bandwidth Wings -->
                {#if isSel}
                  <line x1={nx - qWidth} y1={ny} x2={nx + qWidth} y2={ny} stroke={node.color} stroke-width="2" stroke-dasharray="2 2" />
                  <circle cx={nx - qWidth} cy={ny} r="3.5" fill={node.color} />
                  <circle cx={nx + qWidth} cy={ny} r="3.5" fill={node.color} />
                {/if}

                <!-- Draggable Node Circle -->
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <circle 
                  cx={nx} 
                  cy={ny} 
                  r={isSel ? "8" : "6"} 
                  fill={node.enabled ? node.color : "#555555"} 
                  stroke="#ffffff" 
                  stroke-width={isSel ? "2.5" : "1.5"}
                  class="eq-node-handle"
                  on:click={() => selectedEqNodeId = node.id}
                  on:contextmenu={(e) => {
                    e.preventDefault();
                    selectedEqNodeId = node.id;
                    eqFilterMenuTargetNode = node;
                    eqFilterMenuX = e.clientX;
                    eqFilterMenuY = e.clientY;
                    showEqFilterMenu = true;
                  }}
                />

                <!-- Node Label Pill -->
                <text x={nx} y={ny - 12} text-anchor="middle" fill="#ffffff" font-size="10" font-weight="700">
                  {node.freq >= 1000 ? (node.freq / 1000).toFixed(1) + "k" : Math.round(node.freq)}Hz
                </text>
              {/each}
            </svg>

            <!-- Frequency Axis Legend -->
            <div class="eq-freq-ruler">
              <span>20 Hz</span>
              <span>100 Hz</span>
              <span>500 Hz</span>
              <span>1 kHz</span>
              <span>5 kHz</span>
              <span>10 kHz</span>
              <span>20 kHz</span>
            </div>
          </div>

          <!-- Bottom Parameter Deck for Selected Node -->
          {#if selEqNode}
            <div class="selected-eq-node-deck">
              <!-- Node Tabs Selector -->
              <div class="eq-node-pills-row">
                {#each eqNodes as node, idx}
                  <button 
                    class="eq-node-pill-btn" 
                    class:active={selectedEqNodeId === node.id}
                    style="--node-color: {node.color};"
                    on:click={() => selectedEqNodeId = node.id}
                  >
                    <span class="pill-dot" style="background-color: {node.color};"></span>
                    <span>{idx + 1}. {node.name}</span>
                  </button>
                {/each}
              </div>

              <!-- Node Parameters Control Grid -->
              <div class="eq-node-controls-grid">
                <!-- Filter Type -->
                <div class="eq-ctrl-group">
                  <label class="eq-ctrl-label" for="eq-filter-type">Filter Type</label>
                  <select 
                    id="eq-filter-type"
                    class="eq-ctrl-select" 
                    bind:value={selEqNode.filterType} 
                    on:change={() => updateEqEngine()}
                  >
                    <option value="Peaking">Parametric Bell (Peaking)</option>
                    <option value="LowShelf">Low Shelf</option>
                    <option value="HighShelf">High Shelf</option>
                    <option value="HighPass">High Pass (Low Cut)</option>
                    <option value="LowPass">Low Pass (High Cut)</option>
                    <option value="Notch">Notch (Band Stop)</option>
                  </select>
                </div>

                <!-- Frequency -->
                <div class="eq-ctrl-group">
                  <div class="eq-label-val-row">
                    <label class="eq-ctrl-label" for="eq-freq-input">Frequency</label>
                    <span class="eq-val-badge">{selEqNode.freq >= 1000 ? (selEqNode.freq / 1000).toFixed(2) + " kHz" : selEqNode.freq.toFixed(0) + " Hz"}</span>
                  </div>
                  <input 
                    id="eq-freq-input"
                    type="range" 
                    min="20" 
                    max="20000" 
                    step="1" 
                    bind:value={selEqNode.freq} 
                    on:input={() => updateEqEngine()}
                    class="rack-h-slider" 
                  />
                </div>

                <!-- Gain (for Bell and Shelves) -->
                {#if selEqNode.filterType === 'Peaking' || selEqNode.filterType === 'LowShelf' || selEqNode.filterType === 'HighShelf'}
                  <div class="eq-ctrl-group">
                    <div class="eq-label-val-row">
                      <label class="eq-ctrl-label" for="eq-gain-input">Gain</label>
                      <span class="eq-val-badge">{selEqNode.gainDb > 0 ? "+" : ""}{selEqNode.gainDb.toFixed(1)} dB</span>
                    </div>
                    <input 
                      id="eq-gain-input"
                      type="range" 
                      min="-24.0" 
                      max="24.0" 
                      step="0.5" 
                      bind:value={selEqNode.gainDb} 
                      on:input={() => updateEqEngine()}
                      class="rack-h-slider" 
                    />
                  </div>
                {/if}

                <!-- Q / Bandwidth -->
                <div class="eq-ctrl-group">
                  <div class="eq-label-val-row">
                    <label class="eq-ctrl-label" for="eq-q-input">Q (Bandwidth)</label>
                    <span class="eq-val-badge">Q: {selEqNode.q.toFixed(2)}</span>
                  </div>
                  <input 
                    id="eq-q-input"
                    type="range" 
                    min="0.1" 
                    max="10.0" 
                    step="0.05" 
                    bind:value={selEqNode.q} 
                    on:input={() => updateEqEngine()}
                    class="rack-h-slider" 
                  />
                </div>

                <!-- Actions: Enable / Delete -->
                <div class="eq-ctrl-group actions-group">
                  <button 
                    class="eq-node-power-btn" 
                    class:active={selEqNode.enabled}
                    on:click={() => { selEqNode.enabled = !selEqNode.enabled; updateEqEngine(); }}
                  >
                    {selEqNode.enabled ? "ACTIVE" : "MUTED"}
                  </button>

                  {#if eqNodes.length > 1}
                    <button 
                      class="eq-delete-node-btn" 
                      on:click={() => {
                        eqNodes = eqNodes.filter(n => n.id !== selEqNode.id);
                        selectedEqNodeId = eqNodes[0].id;
                        updateEqEngine();
                      }}
                      title="Delete this filter band"
                    >
                      Delete Band
                    </button>
                  {/if}
                </div>
              </div>
            </div>
          {/if}
        </div>

        <div class="modal-footer">
          <span class="footer-hint">Interactive multi-filter biquad equalization active</span>
          <button class="modal-action-btn" on:click={() => showAdvancedEqModal = false}>Close</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- EQ Node Right-Click Context Menu -->
  {#if showEqFilterMenu && eqFilterMenuTargetNode}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div 
      class="context-menu" 
      style="top: {eqFilterMenuY}px; left: {eqFilterMenuX}px;"
      on:click|stopPropagation
    >
      <div class="menu-item font-semibold" style="color: #64d2ff; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 4px; margin-bottom: 4px;">
        FILTER TYPE: {eqFilterMenuTargetNode.name}
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'Peaking'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'Peaking' ? '✓ ' : '  '}Parametric Bell
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'LowShelf'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'LowShelf' ? '✓ ' : '  '}Low Shelf
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'HighShelf'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'HighShelf' ? '✓ ' : '  '}High Shelf
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'HighPass'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'HighPass' ? '✓ ' : '  '}High Pass (Low Cut)
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'LowPass'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'LowPass' ? '✓ ' : '  '}Low Pass (High Cut)
      </div>
      <div class="menu-item" on:click={() => { if (eqFilterMenuTargetNode) eqFilterMenuTargetNode.filterType = 'Notch'; updateEqEngine(); showEqFilterMenu = false; }}>
        {eqFilterMenuTargetNode.filterType === 'Notch' ? '✓ ' : '  '}Notch Filter
      </div>
    </div>
  {/if}

  <!-- Export Audio Modal -->
  {#if showExportModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => { if (!isExporting) showExportModal = false; }}>
      <div class="inspector-modal export-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge export-badge">EXPORT</span>
            <h3>Export Audio File</h3>
            <span class="stage-subhead">Offline DSP Render Engine</span>
          </div>
          <button class="modal-close-btn" disabled={isExporting} on:click={() => showExportModal = false}>×</button>
        </div>

        <div class="modal-body export-modal-body">
          <div class="export-section">
            <div class="export-section-title">AUDIO FORMAT & BIT DEPTH</div>
            <div class="export-options-grid">
              <label class="export-radio-btn" class:selected={exportBitDepth === 'int16'}>
                <input type="radio" name="bitdepth" value="int16" bind:group={exportBitDepth} />
                <span class="radio-title">WAV (16-bit PCM)</span>
                <span class="radio-desc">CD Quality • Universal compatibility</span>
              </label>
              <label class="export-radio-btn" class:selected={exportBitDepth === 'int24'}>
                <input type="radio" name="bitdepth" value="int24" bind:group={exportBitDepth} />
                <span class="radio-title">WAV (24-bit PCM)</span>
                <span class="radio-desc">Studio High-Resolution (Recommended)</span>
              </label>
              <label class="export-radio-btn" class:selected={exportBitDepth === 'float32'}>
                <input type="radio" name="bitdepth" value="float32" bind:group={exportBitDepth} />
                <span class="radio-title">WAV (32-bit Float)</span>
                <span class="radio-desc">Full 32-bit Floating Point Headroom</span>
              </label>
            </div>
          </div>

          <div class="export-section">
            <div class="export-section-title">EXPORT RANGE</div>
            <div class="export-range-row">
              <label class="export-radio-pill" class:active={exportRange === 'full'}>
                <input type="radio" name="range" value="full" bind:group={exportRange} />
                Full Song ({formatTime(duration)})
              </label>
              <label class="export-radio-pill" class:active={exportRange === 'selection'} class:disabled={!timeSelection}>
                <input type="radio" name="range" value="selection" bind:group={exportRange} disabled={!timeSelection} />
                Time Selection {#if timeSelection}({formatTime(timeSelection.start)} – {formatTime(timeSelection.end)}){:else}(No selection){/if}
              </label>
              <label class="export-radio-pill" class:active={exportRange === 'region'} class:disabled={regions.length === 0}>
                <input type="radio" name="range" value="region" bind:group={exportRange} disabled={regions.length === 0} />
                Specific Region ({regions.length})
              </label>
            </div>
            {#if exportRange === 'region' && regions.length > 0}
              <div class="region-select-wrap">
                <select class="export-select" bind:value={exportSelectedRegionId}>
                  {#each regions as r}
                    <option value={r.id}>{r.name} ({formatTime(r.startTime)} – {formatTime(r.endTime)})</option>
                  {/each}
                </select>
              </div>
            {/if}
          </div>

          <div class="export-section">
            <div class="export-section-title">BAKED PROCESSING OPTIONS</div>
            <div class="export-checkboxes-grid">
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportBakePitch} />
                <span class="check-label">
                  Pitch Shift
                  <span class="check-sub">{pitch > 0 ? "+" : ""}{pitch} st, {pitchCents > 0 ? "+" : ""}{pitchCents}¢</span>
                </span>
              </label>
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportBakeSpeed} />
                <span class="check-label">
                  Playback Speed
                  <span class="check-sub">{speed.toFixed(2)}x tempo</span>
                </span>
              </label>
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportBakeEq} />
                <span class="check-label">
                  Equalizer
                  <span class="check-sub">{isEqBypassed ? 'Bypassed' : 'Active Biquad Cascade'}</span>
                </span>
              </label>
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportBakeCompressor} />
                <span class="check-label">
                  Dynamic Compressor
                  <span class="check-sub">{isCompressorBypassed ? 'Bypassed' : 'Dual-Stage'}</span>
                </span>
              </label>
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportBakeCuts} />
                <span class="check-label">
                  Cut Regions
                  <span class="check-sub">Removed with micro-crossfades</span>
                </span>
              </label>
              <label class="export-check-item">
                <input type="checkbox" bind:checked={exportCopyMetadata} />
                <span class="check-label">
                  Preserve Tags
                  <span class="check-sub">Title, Artist, Album, Year</span>
                </span>
              </label>
            </div>
          </div>

          {#if exportStatusMessage}
            <div class="export-feedback-msg" class:export-success={exportStatusMessage.startsWith('✓')}>
              {exportStatusMessage}
            </div>
          {/if}
          {#if exportErrorMessage}
            <div class="export-feedback-msg export-error">
              {exportErrorMessage}
            </div>
          {/if}
        </div>

        <div class="modal-footer">
          <button class="modal-action-btn cancel-btn" disabled={isExporting} on:click={() => showExportModal = false}>
            Cancel
          </button>
          <button class="modal-action-btn primary-btn export-run-btn" disabled={isExporting} on:click={executeAudioExport}>
            {isExporting ? "Rendering..." : "Choose Location & Export..."}
          </button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Show Control & Remotes Settings Modal (Milestone 8) -->
  {#if showRemoteSettingsModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => showRemoteSettingsModal = false}>
      <div class="inspector-modal remote-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge remote-badge">REMOTE</span>
            <h3>Show Control & Remote Hardware</h3>
            <span class="stage-subhead">Stream Deck • OSC • MIDI</span>
          </div>
          <button class="modal-close-btn" on:click={() => showRemoteSettingsModal = false}>×</button>
        </div>

        <div class="modal-body remote-modal-body">
          <!-- WebSocket / Stream Deck Section -->
          <div class="export-section">
            <div class="remote-section-head">
              <div class="export-section-title">WEBSOCKET SERVER (STREAM DECK & COMPANION)</div>
              <span class="remote-status-badge running-badge">● RUNNING</span>
            </div>
            <div class="remote-info-box">
              <div class="remote-endpoint-row">
                <span class="endpoint-label">Local WebSocket Endpoint:</span>
                <code class="endpoint-val">ws://127.0.0.1:4545</code>
              </div>
              <p class="remote-desc-text">
                Connect your <strong>Elgato Stream Deck</strong> (via Dorico/WebSocket plugin) or <strong>Bitfocus Companion</strong> to control TrackHelm with two-way LCD state updates.
              </p>
              <div class="quick-commands-grid">
                <span class="cmd-pill" title="Play/Pause Toggle"><code>play_pause</code></span>
                <span class="cmd-pill" title="Stop & Rewind / Play Highlighted"><code>rewind</code></span>
                <span class="cmd-pill" title="Next Song in Playlist"><code>next_track</code></span>
                <span class="cmd-pill" title="Previous Song in Playlist"><code>prev_track</code></span>
                <span class="cmd-pill" title="Jump to Next Landmark"><code>next_marker</code></span>
                <span class="cmd-pill" title="Jump to Previous Landmark"><code>prev_marker</code></span>
                <span class="cmd-pill" title="Add Landmark at Playhead"><code>add_marker</code></span>
                <span class="cmd-pill" title="Transpose Pitch +1 st"><code>pitch_up</code> / <code>pitch_down</code></span>
                <span class="cmd-pill" title="Adjust Volume +1 dB"><code>volume_up</code> / <code>volume_down</code></span>
                <span class="cmd-pill" title="Adjust Playback Speed"><code>speed_up</code> / <code>speed_down</code></span>
              </div>
            </div>
          </div>

          <!-- OSC Show Control Section -->
          <div class="export-section">
            <div class="remote-section-head">
              <div class="export-section-title">OSC PROTOCOL (QLAB & DIGITAL CONSOLES)</div>
              <span class="remote-status-badge running-badge">● UDP 4546</span>
            </div>
            <div class="remote-info-box">
              <div class="remote-endpoint-row">
                <span class="endpoint-label">Listening Port:</span>
                <code class="endpoint-val">0.0.0.0:4546 (UDP)</code>
              </div>
              <div class="osc-path-examples">
                <span class="osc-path"><code>/trackhelm/playpause</code></span>
                <span class="osc-path"><code>/trackhelm/rewind</code></span>
                <span class="osc-path"><code>/trackhelm/track/next</code></span>
                <span class="osc-path"><code>/trackhelm/marker/next</code></span>
                <span class="osc-path"><code>/trackhelm/pitch/inc</code></span>
                <span class="osc-path"><code>/trackhelm/volume/inc</code></span>
              </div>
            </div>
          </div>

          <!-- MIDI Hardware Section -->
          <div class="export-section">
            <div class="remote-section-head">
              <div class="export-section-title">HARDWARE MIDI CONTROLLERS & PEDALS</div>
              <button class="mini-refresh-btn" on:click={refreshMidiPorts}>🔄 Rescan</button>
            </div>
            <div class="midi-connect-row">
              <select class="export-select" bind:value={selectedMidiPort}>
                {#if midiPorts.length === 0}
                  <option value="">No MIDI input devices detected</option>
                {:else}
                  {#each midiPorts as port}
                    <option value={port}>{port}</option>
                  {/each}
                {/if}
              </select>
              <button class="modal-action-btn connect-btn" disabled={!selectedMidiPort} on:click={() => connectToMidiPort(selectedMidiPort)}>
                Connect Port
              </button>
            </div>
            {#if midiStatusMessage}
              <div class="midi-status-feedback">{midiStatusMessage}</div>
            {/if}
            <div class="midi-note-map-hint">
              <strong>Default MIDI Notes:</strong> Note 60 (C4) = Play/Pause • Note 62 (D4) = Rewind • Note 64 (E4) = Next Marker • Note 65 (F4) = Prev Marker • Note 67 (G4) = Add Marker • CC 7 = Volume
            </div>
          </div>
        </div>

        <div class="modal-footer">
          <button class="modal-action-btn" on:click={() => showRemoteSettingsModal = false}>
            Done
          </button>
        </div>
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

  /* Grid layout spanning 3-column workspaces (Full 100vh Height) */
  .workspace-grid {
    display: grid;
    grid-template-columns: 240px 1fr 280px;
    flex-grow: 1;
    overflow: hidden;
    height: 100vh;
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
    padding: 1px 8px;
    font-size: 0.72rem;
    cursor: pointer;
    color: #ffffff; /* White text */
    transition: background-color 0.15s ease;
    min-width: 0;
  }

  .browser-item .item-name {
    flex-grow: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
    padding: 1px 10px;
    font-size: 0.72rem;
    cursor: pointer;
    color: #cccccc;
    transition: background-color 0.15s ease;
    min-width: 0;
  }

  .playlist-item-sidebar .item-name {
    flex-grow: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .playlist-item-sidebar:hover {
    background-color: #2d2d2e;
  }

  .playlist-item-sidebar.highlighted {
    background-color: #243547;
    outline: 1px solid rgba(59, 153, 252, 0.7);
    color: #ffffff;
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
    padding: 10px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow: hidden;
    height: 100%;
    box-sizing: border-box;
  }

  .track-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid #28282a;
    padding-bottom: 4px;
    flex-shrink: 0;
  }

  .track-title-info {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
  }

  .track-badge {
    font-size: 0.65rem;
    font-weight: 850;
    letter-spacing: 0.05em;
    padding: 2px 6px;
    border-radius: 3px;
    background-color: #ff9500;
    color: #000000;
    flex-shrink: 0;
  }

  .track-badge.main-badge {
    background-color: #3b99fc;
    color: #ffffff;
  }

  .track-title-text {
    font-size: 0.95rem;
    font-weight: 600;
    color: #ffffff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .time-readout {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-family: Menlo, Monaco, Consolas, monospace;
    background-color: #141414;
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid #2d2d2d;
    flex-shrink: 0;
  }

  .time-large {
    font-size: 1.1rem;
    font-weight: 700;
    color: #ffffff;
  }

  .time-sep {
    color: #555555;
    font-size: 0.85rem;
  }

  .time-total {
    font-size: 0.85rem;
    color: #8e8e8e;
  }

  /* Waveforms stretching wrapper */
  .waveforms-flexbox {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    gap: 6px;
    overflow: hidden;
  }

  .waveform-block {
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .block-overview {
    flex-shrink: 0;
    height: 40px;
  }

  .overview-watermark-tag {
    position: absolute;
    top: 2px;
    left: 4px;
    font-size: 0.6rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    line-height: 1;
    padding: 0;
    background: transparent;
    border: none;
    pointer-events: none;
    z-index: 2;
  }

  .overview-watermark-tag.alt-tag {
    color: #ff9500;
  }

  .overview-watermark-tag.main-tag {
    color: #3b99fc;
  }

  .block-main-waveform {
    flex-shrink: 0;
    height: 256px;
    min-height: 256px;
    max-height: 256px;
    background-color: #122a3a; 
    border: 1px solid #1b384d;
    border-radius: 4px;
  }

  .overview-canvas {
    display: block;
    width: 100%;
    height: 40px;
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
    gap: 10px;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .control-group {
    background-color: #222223;
    border: 1px solid #363638;
    border-radius: 4px;
    padding: 4px 10px;
  }

  .transport-group {
    flex: 0 0 auto;
  }

  .loop-group {
    flex: 1;
  }

  .btn-row {
    display: flex;
    gap: 6px;
  }

  .control-btn {
    background-color: #303032;
    color: #d1d1d1;
    border: 1px solid #424245;
    padding: 5px 12px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.8rem;
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
    font-size: 0.82rem;
  }

  .play-btn:hover {
    background-color: #258bf5;
  }

  .stop-btn {
    background-color: #262628;
    color: #ff6b6b;
    border-color: #4a2222;
    font-size: 0.8rem;
    font-weight: 700;
  }

  .stop-btn:hover {
    background-color: #3d1c1c;
    border-color: #ff6b6b;
    color: #ffffff;
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
    gap: 6px;
    cursor: grab;
    transition: background-color 0.1s ease;
  }

  .marker-item:hover {
    background-color: #383838;
  }

  .marker-item.selected-marker-item {
    background-color: #1e3550;
    border-color: #3b99fc;
    box-shadow: inset 0 0 0 1px #3b99fc;
  }

  .marker-item:active {
    cursor: grabbing;
  }

  .marker-track-mode-badge {
    font-size: 0.65rem;
    font-weight: 700;
    color: #3b99fc;
    margin-left: 4px;
  }

  .marker-track-mode-badge.alt-badge {
    color: #ff9500;
  }

  .unplaced-markers-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.62rem;
    font-weight: 800;
    color: #8e8e8e;
    letter-spacing: 0.05em;
    padding: 8px 2px 3px 2px;
    border-top: 1px dashed #3a3a3d;
    margin-top: 6px;
  }

  .unplaced-hint {
    font-size: 0.58rem;
    color: #3b99fc;
    font-weight: normal;
  }

  .unplaced-main-marker {
    background-color: #222224;
    border-color: #333336;
    opacity: 0.85;
  }

  .unplaced-main-marker:hover {
    background-color: #2b2b2f;
    opacity: 1;
    border-color: #4a4a50;
  }

  .unplaced-name {
    color: #b0b0b8;
  }

  .marker-drag-hint {
    font-size: 0.62rem;
    color: #ff9500;
    font-weight: 600;
    white-space: nowrap;
  }

  .marker-pdf-tag {
    font-size: 0.6rem;
    background: rgba(59, 153, 252, 0.2);
    color: #3b99fc;
    border: 1px solid rgba(59, 153, 252, 0.4);
    border-radius: 8px;
    padding: 1px 5px;
    font-family: monospace;
    white-space: nowrap;
  }

  .marker-color-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    cursor: pointer;
    flex-shrink: 0;
    border: 1px solid rgba(255, 255, 255, 0.4);
    transition: transform 0.1s ease;
  }

  .marker-color-dot:hover {
    transform: scale(1.3);
  }

  .marker-name-btn {
    cursor: pointer;
    font-weight: 600;
    color: #ffffff;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex-grow: 1;
  }

  .marker-time-tag {
    font-size: 0.65rem;
    font-weight: normal;
    color: #8e8e8e;
    margin-left: 3px;
  }

  .marker-name-btn:hover {
    color: #3b99fc;
  }

  .marker-rename-input {
    flex-grow: 1;
    background: #141414;
    border: 1px solid #3b99fc;
    color: #ffffff;
    font-size: 0.75rem;
    padding: 1px 4px;
    border-radius: 2px;
    outline: none;
  }

  .marker-item-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .marker-action-btn {
    background: transparent;
    border: none;
    cursor: pointer;
    font-size: 0.75rem;
    padding: 0 2px;
    opacity: 0.6;
  }

  .marker-action-btn:hover {
    opacity: 1;
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

  /* Lower Rehearsal Deck (Notes, Lyrics, Metadata, PDF) */
  .rehearsal-bottom-deck {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background-color: #161617;
    border: 1px solid #2d2d2d;
    border-radius: 4px;
    overflow: hidden;
  }

  .deck-tabs-header {
    display: flex;
    background-color: #1a1a1c;
    border-bottom: 1px solid #262628;
    flex-shrink: 0;
  }

  .deck-tab-btn {
    background: transparent;
    border: none;
    color: #8e8e8e;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    padding: 3px 12px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: all 0.15s ease;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 280px;
  }

  .deck-tab-btn:hover {
    color: #d1d1d1;
    background-color: #222225;
  }

  .deck-tab-btn.active {
    color: #ffffff;
    background-color: #28282b;
    border-bottom: 2px solid #3b99fc;
  }

  .deck-tab-btn.pdf-tab-btn {
    color: #a8d1ff;
  }

  .deck-tab-btn.pdf-tab-btn.active {
    color: #ffffff;
  }

  .deck-tab-content {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  .tab-pane {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    box-sizing: border-box;
  }

  .notes-pane, .lyrics-pane {
    padding: 6px;
  }

  .rehearsal-textarea {
    flex-grow: 1;
    width: 100%;
    height: 100%;
    background-color: #0f0f10;
    border: 1px solid #262628;
    border-radius: 4px;
    color: #e0e0e0;
    font-family: inherit;
    font-size: 0.85rem;
    line-height: 1.5;
    padding: 8px 12px;
    box-sizing: border-box;
    resize: none;
    outline: none;
  }

  .rehearsal-textarea:focus {
    border-color: #3b99fc;
    background-color: #121214;
  }

  .lyrics-textarea {
    font-family: Menlo, Monaco, Consolas, monospace;
    font-size: 0.85rem;
    line-height: 1.6;
  }

  .metadata-split-layout {
    display: flex;
    gap: 14px;
    height: 100%;
    overflow-y: auto;
    min-height: 0;
  }

  .metadata-edit-section {
    flex: 1.6;
    display: flex;
    flex-direction: column;
    background-color: #141415;
    border: 1px solid #28282b;
    border-radius: 4px;
    padding: 8px 12px;
    overflow-y: auto;
    min-width: 0;
  }

  .metadata-specs-section {
    flex: 1;
    display: flex;
    flex-direction: column;
    background-color: #18181a;
    border: 1px solid #28282b;
    border-radius: 4px;
    padding: 8px 12px;
    overflow-y: auto;
    min-width: 0;
  }

  .meta-section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
    flex-wrap: wrap;
    gap: 6px;
  }

  .meta-section-title {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #8e8e8e;
  }

  .meta-save-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .tag-save-feedback {
    font-size: 0.72rem;
    font-weight: 600;
    color: #34c759;
  }

  .save-tags-btn {
    background-color: #3b99fc;
    color: #ffffff;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  .save-tags-btn:hover:not(:disabled) {
    background-color: #258bf5;
  }

  .save-tags-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .meta-form-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 10px;
  }

  .meta-field-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .meta-field-group.full {
    grid-column: 1 / -1;
  }

  .meta-field-row {
    grid-column: 1 / -1;
    display: flex;
    gap: 10px;
  }

  .meta-field-group.half {
    flex: 1;
  }

  .meta-field-label {
    font-size: 0.65rem;
    font-weight: 600;
    color: #7b9bb6;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .meta-input {
    background-color: #0c0c0d;
    border: 1px solid #2d2d30;
    border-radius: 3px;
    color: #ffffff;
    font-size: 0.78rem;
    padding: 4px 6px;
    outline: none;
    transition: border-color 0.15s ease;
  }

  .meta-input:focus {
    border-color: #3b99fc;
    background-color: #121214;
  }

  .specs-grid {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 6px;
  }

  .spec-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.72rem;
    padding: 2px 0;
    border-bottom: 1px solid #222225;
  }

  .spec-label {
    color: #8e8e8e;
    font-weight: 600;
  }

  .spec-val {
    color: #d1d1d1;
    font-family: monospace;
    font-size: 0.72rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 150px;
  }

  .spec-val.path-val {
    max-width: 130px;
    font-size: 0.68rem;
  }

  .spec-val.active-slot {
    color: #3b99fc;
    font-weight: bold;
  }

  .pdf-viewer-container {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .pdf-floating-controls {
    position: absolute;
    top: 8px;
    right: 14px;
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 6px;
    background-color: rgba(22, 22, 25, 0.85);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    padding: 2px 8px;
    border-radius: 16px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }

  .pdf-page-pill {
    font-size: 0.65rem;
    font-weight: 700;
    color: #a0a0a8;
    font-family: monospace;
    padding-right: 4px;
  }

  .pdf-mini-btn {
    background: #2a2a2e;
    color: #d1d1d1;
    border: 1px solid #444448;
    border-radius: 10px;
    padding: 1px 7px;
    font-size: 0.65rem;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .pdf-mini-btn:hover {
    background-color: #383840;
    color: #ffffff;
  }

  .pdf-mini-btn.active-toggle {
    background-color: #ff9500;
    border-color: #ff9500;
    color: #000000;
    font-weight: 700;
  }

  .pdf-mini-btn.active-toggle:hover {
    background-color: #ffaa33;
  }

  .pdf-mini-btn.popout {
    background-color: #3b99fc;
    border-color: #3b99fc;
    color: #ffffff;
    font-weight: 600;
  }

  .pdf-mini-btn.popout:hover {
    background-color: #5faeff;
  }

  .pdf-loading-overlay {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 16px;
    color: #a0a0a8;
    font-size: 0.8rem;
    font-weight: 500;
  }

  .pdf-loading-spinner {
    font-size: 1.1rem;
    animation: pulse 1s infinite alternate;
  }

  @keyframes pulse {
    from { opacity: 0.4; }
    to { opacity: 1; }
  }

  .pdf-error-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    background-color: #331515;
    border: 1px solid #662222;
    border-radius: 4px;
    padding: 8px 12px;
    color: #ff8080;
    font-size: 0.75rem;
  }

  .retry-pdf-btn {
    background-color: #662222;
    color: #ffffff;
    border: none;
    border-radius: 3px;
    padding: 2px 8px;
    cursor: pointer;
    font-size: 0.7rem;
  }

  /* Single scrollable column of full-width PDF pages */
  .pdf-scroll-column {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 12px 16px 24px 16px;
    min-height: 0;
    width: 100%;
    box-sizing: border-box;
    background-color: #121214;
    border-radius: 4px;
  }

  :global(.pdf-page-card) {
    position: relative;
    flex-shrink: 0 !important;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background-color: #ffffff;
    border-radius: 4px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    overflow: visible;
    margin: 0 auto;
    box-sizing: border-box;
    transition: filter 0.2s ease, background-color 0.2s ease;
  }

  :global(.pdf-page-card.inverted) {
    background-color: #000000 !important;
    border: 1px solid #28282c;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.85);
  }

  :global(.pdf-page-card.inverted .pdf-page-canvas) {
    filter: invert(1) hue-rotate(180deg);
  }

  :global(.pdf-marker-badge) {
    position: absolute;
    transform: translate(-10px, -50%);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    border-radius: 12px;
    font-size: 0.7rem;
    font-weight: 700;
    color: #ffffff;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.6);
    cursor: grab;
    z-index: 10;
    user-select: none;
    -webkit-user-select: none;
    border: 1.5px solid rgba(255, 255, 255, 0.35);
    white-space: nowrap;
    transition: transform 0.1s ease, box-shadow 0.1s ease;
  }

  :global(.pdf-marker-badge:hover) {
    transform: translate(-10px, -50%) scale(1.08);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.85);
  }

  :global(.pdf-marker-badge:active) {
    cursor: grabbing;
  }

  :global(.marker-pointer-drag-ghost) {
    position: fixed;
    pointer-events: none;
    z-index: 99999;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    border-radius: 12px;
    font-size: 0.7rem;
    font-weight: 700;
    color: #ffffff;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.65);
    border: 1.5px solid rgba(255, 255, 255, 0.4);
    white-space: nowrap;
    opacity: 0.85;
    transform: translate(12px, 12px);
    transition: transform 0.12s cubic-bezier(0.16, 1, 0.3, 1), box-shadow 0.12s ease, border-color 0.12s ease, opacity 0.12s ease;
  }

  :global(.marker-pointer-drag-ghost.on-pdf-preview) {
    transform: translate(-10px, -50%) scale(1.08);
    opacity: 1;
    box-shadow: 0 4px 18px rgba(0, 0, 0, 0.85);
    border: 1.5px solid rgba(255, 255, 255, 0.85);
  }

  :global(.pdf-marker-dot) {
    width: 6px;
    height: 6px;
    background-color: #ffffff;
    border-radius: 50%;
  }

  :global(.pdf-marker-title) {
    pointer-events: none;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }

  :global(.pdf-marker-unpin) {
    background: rgba(0, 0, 0, 0.45);
    border: none;
    color: #ffffff;
    border-radius: 50%;
    width: 0;
    height: 13px;
    line-height: 12px;
    font-size: 0.65rem;
    text-align: center;
    cursor: pointer;
    padding: 0;
    margin-left: 0;
    opacity: 0;
    pointer-events: none;
    overflow: hidden;
    transition: opacity 0.15s ease, width 0.15s ease, margin-left 0.15s ease, background 0.1s ease;
  }

  :global(.pdf-marker-badge:hover .pdf-marker-unpin) {
    opacity: 1;
    pointer-events: auto;
    width: 13px;
    margin-left: 3px;
  }

  :global(.pdf-marker-unpin:hover) {
    background: rgba(255, 59, 48, 0.9);
  }

  :global(.pdf-page-canvas) {
    display: block;
    box-sizing: border-box;
  }

  .no-pdf-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #717171;
    gap: 6px;
    padding: 12px;
    text-align: center;
  }

  .pdf-placeholder-icon {
    font-size: 1.8rem;
  }

  .link-pdf-btn {
    background-color: #2c2c2e;
    border: 1px dashed #4a4a4e;
    color: #d1d1d1;
    padding: 5px 12px;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .link-pdf-btn:hover {
    background-color: #3b99fc;
    border-color: #3b99fc;
    color: #ffffff;
  }

  /* DSP Panel Knobs styling */
  .dsp-section {
    padding-bottom: 2px;
  }

  .dsp-control {
    padding: 2px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dsp-label-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.68rem;
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
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    justify-items: center;
    align-items: center;
    padding: 6px 2px;
    background-color: #171719;
    width: 100%;
    box-sizing: border-box;
  }

  .knob-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    cursor: ns-resize;
    width: 100%;
    max-width: 70px;
  }

  .knob-label {
    font-size: 0.58rem;
    color: #8e8e8e;
    margin-bottom: 2px;
    text-align: center;
    white-space: nowrap;
  }

  .knob-circle {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background-color: #2b2b2e;
    border: 1.5px solid #4a4a50;
    position: relative;
    box-shadow: inset 0 2px 4px rgba(0,0,0,0.5);
    margin-bottom: 2px;
    transition: border-color 0.15s ease;
  }

  .knob-zero-tick {
    position: absolute;
    top: 1px;
    left: calc(50% - 0.75px);
    width: 1.5px;
    height: 3px;
    background-color: rgba(255, 255, 255, 0.4);
    border-radius: 1px;
    pointer-events: none;
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
    font-size: 0.62rem;
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
    padding: 7px 14px;
    font-size: 0.76rem;
    color: #d1d1d1;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .menu-item:hover {
    background-color: #3b99fc;
    color: #ffffff;
  }

  .menu-item.menu-active {
    color: #3b99fc;
    font-weight: 700;
  }

  .menu-item.menu-active:hover {
    color: #ffffff;
  }

  .menu-item.menu-item-danger:hover {
    background-color: #662222;
    color: #ff9999;
  }

  .menu-divider {
    height: 1px;
    background-color: #3e3e42;
    margin: 4px 0;
  }

  /* Marker Color Palette Popover */
  .marker-color-palette-popover {
    position: fixed;
    z-index: 100000;
    background-color: #202024;
    border: 1px solid #44444a;
    border-radius: 8px;
    padding: 8px 10px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.75), 0 2px 6px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    gap: 6px;
    user-select: none;
    animation: fadeInScale 0.12s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes fadeInScale {
    from { opacity: 0; transform: scale(0.92) translateY(-4px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .palette-header {
    font-size: 0.65rem;
    font-weight: 700;
    color: #8e8e96;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .palette-grid {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 6px;
  }

  .palette-swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid rgba(0, 0, 0, 0.35);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    transition: transform 0.1s ease, box-shadow 0.1s ease, border-color 0.1s ease;
  }

  .palette-swatch:hover {
    transform: scale(1.18);
    box-shadow: 0 0 8px rgba(255, 255, 255, 0.4);
    border-color: #ffffff;
  }

  .palette-swatch.selected {
    box-shadow: 0 0 0 2px #3b99fc, 0 0 8px rgba(59, 153, 252, 0.6);
    border-color: #ffffff;
  }

  .swatch-check {
    font-size: 0.65rem;
    font-weight: 900;
    color: #000000;
    text-shadow: 0 0 2px rgba(255, 255, 255, 0.8);
  }

  /* Vertical 90° Module Tab Buttons & Effects Layout */
  .effects-module-row {
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid #232326;
    background-color: #171719;
  }

  .module-tab-btn {
    width: 22px;
    background-color: #007aff;
    border: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    transition: all 0.15s ease;
    user-select: none;
    flex-shrink: 0;
  }

  .module-tab-btn:hover {
    background-color: #0088ff;
    filter: brightness(1.2);
  }

  .module-tab-btn span {
    transform: rotate(-90deg);
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 1px;
    color: #ffffff;
    white-space: nowrap;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .module-tab-btn.comp-btn {
    background-color: #007aff;
  }

  .module-tab-btn.eq-btn {
    background-color: #007aff;
  }

  /* Regions Sidebar Items */
  .regions-header-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 10px 4px;
    background-color: #1a1a1d;
    border-top: 1px solid #29292d;
    border-bottom: 1px solid #29292d;
    margin-top: 6px;
  }

  .regions-header-title {
    font-size: 0.62rem;
    font-weight: 800;
    letter-spacing: 0.05em;
    color: #8e8e96;
  }

  .regions-count {
    font-size: 0.6rem;
    background-color: #2c2c30;
    color: #b0b0b8;
    padding: 1px 5px;
    border-radius: 8px;
  }

  .region-sidebar-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 3px 6px;
    height: 32px;
    box-sizing: border-box;
    border-bottom: 1px solid #1f1f22;
    background-color: #141416;
    cursor: pointer;
    transition: background 0.12s ease;
  }

  .region-sidebar-item:hover {
    background-color: #1c1c20;
  }

  .region-sidebar-item.active-region {
    background-color: #1a2533;
  }

  .region-sidebar-item.is-loop {
    border-left: 3px solid #30d158;
  }

  .region-sidebar-item.is-cut {
    border-left: 3px solid #ff453a;
  }

  .region-item-main {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    flex-grow: 1;
    overflow: hidden;
  }

  .region-color-bar {
    width: 3px;
    height: 16px;
    border-radius: 1px;
    flex-shrink: 0;
  }

  .region-item-info {
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
    gap: 1px;
  }

  .region-name {
    font-size: 0.72rem;
    font-weight: 600;
    color: #e2e8f0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.1;
  }

  .region-span {
    font-size: 0.62rem;
    color: #7a7e8a;
    font-family: Menlo, monospace;
    line-height: 1;
  }

  .region-item-toggles {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
  }

  .region-toggle-btn {
    background: transparent;
    border: 1px solid #38383e;
    border-radius: 4px;
    padding: 1px 4px;
    font-size: 0.7rem;
    cursor: pointer;
    opacity: 0.5;
    transition: all 0.12s ease;
  }

  .region-toggle-btn:hover {
    opacity: 0.85;
    background-color: rgba(255, 255, 255, 0.08);
  }

  .region-toggle-btn.active {
    opacity: 1;
    background-color: rgba(48, 209, 88, 0.25);
    border-color: #30d158;
  }

  .region-toggle-btn.cut-toggle-btn.active {
    background-color: rgba(255, 69, 58, 0.25);
    border-color: #ff453a;
  }

  /* Markdown Deck & Chord Badges (Obsidian Dark Styled) */
  .markdown-deck-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .markdown-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 10px;
    background-color: #1a1a1d;
    border-bottom: 1px solid #28282c;
    flex-shrink: 0;
  }

  .md-toolbar-title {
    font-size: 0.65rem;
    font-weight: 800;
    letter-spacing: 0.06em;
    color: #8e8e96;
  }

  .md-view-toggles {
    display: flex;
    gap: 4px;
  }

  .md-toggle-btn {
    background: #242428;
    border: 1px solid #36363c;
    color: #b0b0b8;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 0.68rem;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .md-toggle-btn:hover {
    background: #303038;
    color: #ffffff;
  }

  .md-toggle-btn.active {
    background: #007aff;
    border-color: #0088ff;
    color: #ffffff;
    font-weight: 700;
  }

  .markdown-editor-container {
    flex-grow: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
  }

  .markdown-editor-container.mode-split {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }

  .md-textarea {
    width: 100%;
    height: 100%;
    border: none;
    outline: none;
    resize: none;
    background-color: #141416;
    color: #e0e0e6;
    font-family: "SF Mono", Menlo, Monaco, Consolas, monospace;
    font-size: 0.82rem;
    line-height: 1.5;
    padding: 12px 14px;
    box-sizing: border-box;
    border-right: 1px solid #232326;
  }

  .markdown-preview-pane {
    flex-grow: 1;
    height: 100%;
    overflow-y: auto;
    padding: 12px 16px;
    background-color: #161619;
    color: #d1d1d6;
    box-sizing: border-box;
    font-size: 0.85rem;
    line-height: 1.6;
  }

  .markdown-empty-hint {
    color: #636366;
    font-style: italic;
    padding: 20px 0;
  }

  :global(.chord-badge) {
    display: inline-block;
    background: linear-gradient(135deg, #1c355e, #132442);
    color: #64d2ff;
    border: 1px solid rgba(100, 210, 255, 0.4);
    border-radius: 4px;
    padding: 1px 5px;
    font-weight: 700;
    font-family: "SF Mono", Menlo, monospace;
    font-size: 0.78rem;
    margin: 0 2px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  }

  :global(.md-h1) {
    font-size: 1.25rem;
    font-weight: 800;
    color: #ffffff;
    border-bottom: 1px solid #2e2e34;
    padding-bottom: 4px;
    margin: 8px 0 6px;
  }

  :global(.md-h2) {
    font-size: 1.05rem;
    font-weight: 700;
    color: #3b99fc;
    margin: 8px 0 4px;
  }

  :global(.md-h3) {
    font-size: 0.9rem;
    font-weight: 700;
    color: #ff9500;
    margin: 6px 0 2px;
  }

  :global(.md-quote) {
    border-left: 3px solid #30d158;
    background-color: rgba(48, 209, 88, 0.08);
    margin: 6px 0;
    padding: 4px 10px;
    color: #a1a1aa;
    border-radius: 0 4px 4px 0;
  }

  :global(.md-code) {
    background-color: #242428;
    color: #ffcc00;
    padding: 1px 4px;
    border-radius: 3px;
    font-family: monospace;
    font-size: 0.8rem;
  }

  :global(.md-p) {
    margin: 4px 0;
  }

  /* Center Deck Files Tab */
  .files-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 14px 16px;
    box-sizing: border-box;
    overflow-y: auto;
  }

  .files-pane-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid #2c2c30;
    padding-bottom: 10px;
    margin-bottom: 12px;
  }

  .files-header-left {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .files-pane-title {
    font-size: 0.82rem;
    font-weight: 800;
    letter-spacing: 0.05em;
    color: #ffffff;
  }

  .files-pane-subtitle {
    font-size: 0.68rem;
    color: #8e8e93;
  }

  .add-assoc-file-btn {
    background-color: #007aff;
    color: #ffffff;
    border: none;
    border-radius: 4px;
    padding: 5px 12px;
    font-size: 0.75rem;
    font-weight: 700;
    cursor: pointer;
    transition: background 0.12s ease;
  }

  .add-assoc-file-btn:hover {
    background-color: #0088ff;
  }

  .empty-files-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background-color: #141416;
    border: 1px dashed #36363d;
    border-radius: 8px;
    padding: 36px 20px;
    text-align: center;
    margin-top: 10px;
  }

  .empty-icon {
    font-size: 2.2rem;
    margin-bottom: 8px;
  }

  .empty-files-card h3 {
    margin: 0 0 6px;
    color: #ffffff;
    font-size: 0.95rem;
  }

  .empty-files-card p {
    margin: 0 0 16px;
    color: #8e8e96;
    font-size: 0.78rem;
    max-width: 440px;
  }

  .empty-actions-row {
    display: flex;
    gap: 10px;
  }

  .action-card-btn {
    background-color: #242429;
    border: 1px solid #3d3d45;
    color: #ffffff;
    border-radius: 4px;
    padding: 6px 14px;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .action-card-btn:hover {
    background-color: #32323a;
    border-color: #007aff;
  }

  .assoc-files-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 10px;
  }

  .assoc-card {
    display: flex;
    align-items: center;
    gap: 10px;
    background-color: #18181b;
    border: 1px solid #2b2b30;
    border-radius: 6px;
    padding: 8px 10px;
    transition: border-color 0.15s ease;
  }

  .assoc-card:hover {
    border-color: #3e3e48;
  }

  .assoc-card.pdf-card {
    border-left: 3px solid #ff9500;
  }

  .assoc-card.audio-card {
    border-left: 3px solid #30d158;
  }

  .assoc-card-icon {
    font-size: 1.2rem;
    flex-shrink: 0;
  }

  .assoc-card-info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex-grow: 1;
  }

  .assoc-card-name {
    font-size: 0.78rem;
    font-weight: 700;
    color: #ffffff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .assoc-card-path {
    font-size: 0.62rem;
    color: #71717a;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: monospace;
  }

  .assoc-card-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .assoc-action-btn {
    background-color: #242429;
    border: 1px solid #36363d;
    color: #d1d1d6;
    border-radius: 3px;
    padding: 3px 6px;
    font-size: 0.68rem;
    cursor: pointer;
    font-weight: 600;
    transition: all 0.12s ease;
  }

  .assoc-action-btn:hover {
    background-color: #34343d;
    color: #ffffff;
  }

  .assoc-action-btn.open-pdf-action {
    background-color: rgba(255, 149, 0, 0.15);
    border-color: rgba(255, 149, 0, 0.35);
    color: #ff9500;
  }

  .assoc-action-btn.open-pdf-action:hover {
    background-color: #ff9500;
    color: #000000;
  }

  .assoc-action-btn.load-main-action {
    background-color: rgba(48, 209, 88, 0.15);
    border-color: rgba(48, 209, 88, 0.35);
    color: #30d158;
  }

  .assoc-action-btn.load-main-action:hover {
    background-color: #30d158;
    color: #000000;
  }

  .assoc-action-btn.unlink-action {
    color: #8e8e96;
    padding: 2px 5px;
  }

  .assoc-action-btn.unlink-action:hover {
    color: #ff453a;
    background-color: rgba(255, 69, 58, 0.15);
  }

  .files-badge-count {
    font-size: 0.62rem;
    color: #3b99fc;
    margin-left: 3px;
  }

  .tab-close-btn {
    font-size: 0.9rem;
    color: #8e8e96;
    margin-left: 6px;
    padding: 0 3px;
    border-radius: 50%;
  }

  .tab-close-btn:hover {
    color: #ff453a;
    background: rgba(255, 255, 255, 0.1);
  }

  /* Right Sidebar Effects Module Tab Column & Bypass Buttons */
  .module-tab-col {
    display: flex;
    flex-direction: column;
    width: 24px;
    flex-shrink: 0;
    border-right: 1px solid #2d2d2d;
    background-color: #141416;
  }

  .module-tab-col .module-tab-btn {
    flex-grow: 1;
    width: 100%;
  }

  .module-tab-btn.pitch-btn {
    background-color: #26262a;
    cursor: default;
  }

  .module-tab-btn.pitch-btn:hover {
    background-color: #26262a;
    filter: none;
  }

  .module-bypass-placeholder {
    height: 14px;
    background-color: #141416;
    border-top: 1px solid #2d2d2d;
  }

  .module-bypass-btn {
    background-color: #1f1f23;
    border: none;
    border-top: 1px solid #2d2d2d;
    color: #30d158;
    font-size: 0.55rem;
    font-weight: 900;
    padding: 2px 0;
    cursor: pointer;
    text-align: center;
    transition: all 0.12s ease;
  }

  .module-bypass-btn:hover {
    filter: brightness(1.3);
  }

  .module-bypass-btn.is-bypassed {
    background-color: #3a1c1c;
    color: #ff453a;
  }

  .knobs-row.effect-bypassed {
    opacity: 0.45;
    filter: grayscale(0.8);
  }

  /* Advanced Modal Sizing */
  .advanced-comp-modal {
    width: 640px !important;
  }

  .advanced-eq-modal {
    width: 720px !important;
  }

  .stage-subhead {
    font-size: 0.65rem;
    color: #8e8e96;
    margin-left: 6px;
  }

  .modal-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .comp-stage-tabs, .comp-routing-selector {
    display: flex;
    background: #101012;
    border-radius: 4px;
    padding: 2px;
    border: 1px solid #2e2e34;
  }

  .stage-tab-btn, .routing-btn {
    background: transparent;
    border: none;
    color: #8e8e96;
    font-size: 0.65rem;
    font-weight: 700;
    padding: 3px 7px;
    border-radius: 3px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .stage-tab-btn.active, .routing-btn.active {
    background: #007aff;
    color: #ffffff;
  }

  .modal-byp-btn {
    background: rgba(48, 209, 88, 0.15);
    border: 1px solid #30d158;
    color: #30d158;
    font-size: 0.65rem;
    font-weight: 800;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
  }

  .modal-byp-btn.is-bypassed {
    background: rgba(255, 69, 58, 0.15);
    border-color: #ff453a;
    color: #ff453a;
  }

  /* Sonitus Console Grid & Controls */
  .sonitus-console-grid {
    display: grid;
    grid-template-columns: 50px 1fr 40px 50px 50px;
    gap: 8px;
    background-color: #0c0c0e;
    border: 1px solid #26262c;
    border-radius: 6px;
    padding: 10px;
    align-items: stretch;
  }

  .console-meter-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }

  .meter-label {
    font-size: 0.58rem;
    font-weight: 800;
    color: #8e8e96;
    letter-spacing: 0.04em;
  }

  .meter-slider-combo {
    flex-grow: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 160px;
  }

  .stereo-peak-track {
    display: flex;
    gap: 2px;
    width: 14px;
    height: 100%;
    background-color: #141418;
    border-radius: 2px;
    padding: 1px;
  }

  .peak-channel {
    flex: 1;
    background-color: #181820;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    border-radius: 1px;
    overflow: hidden;
  }

  .peak-fill {
    width: 100%;
    background: linear-gradient(to top, #30d158 0%, #30d158 70%, #ffcc00 85%, #ff453a 100%);
  }

  .gr-peak-track {
    width: 14px;
    height: 100%;
    background-color: #141418;
    border-radius: 2px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    overflow: hidden;
    padding: 1px;
  }

  .gr-fill {
    width: 100%;
    background: linear-gradient(to top, #ffcc00, #ff453a);
  }

  .vert-slider-wrapper {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
  }

  .vert-slider {
    -webkit-appearance: slider-vertical;
    writing-mode: bt-lr;
    width: 16px;
    height: 140px;
    cursor: pointer;
  }

  .meter-val-tag {
    font-size: 0.62rem;
    font-family: monospace;
    color: #a1a1aa;
    font-weight: 700;
  }

  .meter-val-tag.gr-val {
    color: #ffcc00;
  }

  .sonitus-graph-card {
    position: relative;
    background-color: #08080a;
    border: 1px solid #202026;
    border-radius: 4px;
    overflow: hidden;
    height: 180px;
  }

  .sonitus-svg {
    width: 100%;
    height: 100%;
  }

  .graph-axes-labels {
    position: absolute;
    top: 4px;
    left: 6px;
    right: 6px;
    bottom: 4px;
    pointer-events: none;
  }

  .axis-lbl {
    position: absolute;
    font-size: 0.58rem;
    font-family: monospace;
    color: rgba(255, 255, 255, 0.35);
  }

  .axis-lbl.top-left { top: 0; left: 0; }
  .axis-lbl.bottom-left { bottom: 0; left: 0; }
  .axis-lbl.bottom-right { bottom: 0; right: 0; }
  .axis-lbl.curve-type-tag {
    top: 0;
    right: 0;
    font-weight: 800;
    color: #ffcc00;
  }

  .sonitus-params-rack {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: 8px;
    background-color: #121215;
    border: 1px solid #26262c;
    border-radius: 6px;
    padding: 10px;
  }

  .param-knob-box {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .param-header-label {
    font-size: 0.58rem;
    font-weight: 800;
    color: #8e8e96;
  }

  .stage-power-toggle {
    background-color: #242429;
    border: 1px solid #36363d;
    color: #8e8e96;
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 0.72rem;
    font-weight: 700;
    cursor: pointer;
  }

  .stage-power-toggle.active {
    background-color: rgba(48, 209, 88, 0.2);
    border-color: #30d158;
    color: #30d158;
  }

  .comp-type-dropdown {
    background-color: #1c1c20;
    border: 1px solid #36363d;
    color: #ffffff;
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 0.72rem;
    outline: none;
  }

  .param-title-val {
    display: flex;
    justify-content: space-between;
    font-size: 0.65rem;
    color: #a1a1aa;
    font-weight: 600;
  }

  .val-highlight {
    color: #ffcc00;
    font-family: monospace;
    font-weight: 700;
  }

  .rack-h-slider {
    width: 100%;
    accent-color: #007aff;
    cursor: pointer;
  }

  /* Kirchhoff EQ Canvas & Node Controls */
  .kirchhoff-eq-canvas-card {
    position: relative;
    background-color: #08080a;
    border: 1px solid #222228;
    border-radius: 6px;
    overflow: hidden;
  }

  .kirchhoff-eq-svg {
    width: 100%;
    height: 200px;
  }

  .eq-node-handle {
    cursor: pointer;
    transition: r 0.12s ease;
  }

  .eq-node-handle:hover {
    r: 9;
  }

  .eq-freq-ruler {
    display: flex;
    justify-content: space-between;
    padding: 4px 12px;
    background-color: #101014;
    border-top: 1px solid #202026;
    font-size: 0.62rem;
    color: #71717a;
    font-family: monospace;
  }

  .selected-eq-node-deck {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background-color: #121215;
    border: 1px solid #26262c;
    border-radius: 6px;
    padding: 10px;
  }

  .eq-node-pills-row {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 2px;
  }

  .eq-node-pill-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    background-color: #1b1b1f;
    border: 1px solid #2f2f36;
    color: #b0b0b8;
    border-radius: 12px;
    padding: 3px 10px;
    font-size: 0.7rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .eq-node-pill-btn:hover {
    background-color: #282830;
    color: #ffffff;
  }

  .eq-node-pill-btn.active {
    background-color: #2c2c36;
    border-color: var(--node-color);
    color: #ffffff;
    font-weight: 700;
  }

  .pill-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .eq-node-controls-grid {
    display: grid;
    grid-template-columns: 180px 1fr 1fr 1fr 120px;
    gap: 10px;
    align-items: center;
  }

  .eq-ctrl-group {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .eq-ctrl-label {
    font-size: 0.62rem;
    font-weight: 700;
    color: #8e8e96;
  }

  .eq-ctrl-select {
    background-color: #1c1c20;
    border: 1px solid #383842;
    color: #ffffff;
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 0.72rem;
    outline: none;
  }

  .eq-label-val-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .eq-val-badge {
    font-size: 0.65rem;
    font-family: monospace;
    color: #64d2ff;
    font-weight: 700;
  }

  .actions-group {
    display: flex;
    flex-direction: row;
    gap: 6px;
  }

  .eq-node-power-btn {
    background-color: #1c1c20;
    border: 1px solid #383842;
    color: #8e8e96;
    border-radius: 4px;
    padding: 5px 8px;
    font-size: 0.68rem;
    font-weight: 700;
    cursor: pointer;
  }

  .eq-node-power-btn.active {
    background-color: rgba(48, 209, 88, 0.2);
    border-color: #30d158;
    color: #30d158;
  }

  .eq-delete-node-btn {
    background-color: rgba(255, 69, 58, 0.15);
    border: 1px solid rgba(255, 69, 58, 0.4);
    color: #ff453a;
    border-radius: 4px;
    padding: 5px 8px;
    font-size: 0.68rem;
    font-weight: 600;
    cursor: pointer;
  }

  .eq-delete-node-btn:hover {
    background-color: #ff453a;
    color: #ffffff;
  }

  .add-eq-band-btn {
    background-color: #242429;
    border: 1px solid #3b99fc;
    color: #3b99fc;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 0.68rem;
    font-weight: 700;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .add-eq-band-btn:hover {
    background-color: #3b99fc;
    color: #ffffff;
  }

  /* Modals Base */
  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background-color: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(4px);
    z-index: 20000;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fadeIn 0.15s ease-out;
  }

  .inspector-modal {
    background-color: #1a1a1d;
    border: 1px solid #383840;
    border-radius: 10px;
    max-width: 92vw;
    max-height: 90vh;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.85);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: modalScaleUp 0.15s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes modalScaleUp {
    from { opacity: 0; transform: scale(0.95) translateY(8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 16px;
    background-color: #141416;
    border-bottom: 1px solid #28282e;
  }

  .modal-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .modal-title-row h3 {
    margin: 0;
    font-size: 0.92rem;
    font-weight: 700;
    color: #ffffff;
  }

  .modal-badge {
    font-size: 0.65rem;
    font-weight: 900;
    padding: 2px 6px;
    border-radius: 4px;
    color: #ffffff;
  }

  .modal-badge.comp-badge {
    background-color: #007aff;
  }

  .modal-badge.eq-badge {
    background-color: #007aff;
  }

  .modal-close-btn {
    background: transparent;
    border: none;
    color: #8e8e96;
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0 4px;
    transition: color 0.12s ease;
  }

  .modal-close-btn:hover {
    color: #ffffff;
  }

  .modal-body {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
  }

  .modal-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 16px;
    background-color: #141416;
    border-top: 1px solid #28282e;
  }

  .footer-hint {
    font-size: 0.65rem;
    color: #30d158;
  }

  .modal-action-btn {
    background-color: #007aff;
    border: none;
    border-radius: 4px;
    color: #ffffff;
    padding: 5px 14px;
    font-size: 0.78rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s ease;
  }

  .modal-action-btn:hover {
    background-color: #0088ff;
  }

  /* Header Export Button */
  .header-export-wrap {
    display: flex;
    align-items: center;
  }

  .export-audio-header-btn {
    background: linear-gradient(180deg, #2a2d34 0%, #1f2127 100%);
    border: 1px solid #3e424d;
    border-radius: 4px;
    color: #e2e8f0;
    padding: 4px 10px;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 5px;
    transition: all 0.15s ease;
  }

  .export-audio-header-btn:hover:not(:disabled) {
    background: linear-gradient(180deg, #3b404d 0%, #292c35 100%);
    border-color: #3b99fc;
    color: #ffffff;
    box-shadow: 0 0 8px rgba(59, 153, 252, 0.3);
  }

  .export-audio-header-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Playlist Sidebar Buttons */
  .save-set-btn, .open-set-btn {
    background-color: #24252a !important;
    border: 1px solid #3a3b42 !important;
    color: #cfd3dc !important;
  }

  .save-set-btn:hover, .open-set-btn:hover {
    background-color: #2e3037 !important;
    border-color: #555863 !important;
    color: #ffffff !important;
  }

  /* Region Crossfade Pill */
  .region-meta-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .region-xfade-pill {
    background-color: rgba(255, 69, 58, 0.15);
    border: 1px solid rgba(255, 69, 58, 0.35);
    color: #ff6961;
    font-size: 0.62rem;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 3px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .region-xfade-pill:hover {
    background-color: rgba(255, 69, 58, 0.3);
    border-color: #ff453a;
    color: #ffffff;
  }

  /* Export Audio Modal Styles */
  .export-modal-card {
    width: 580px;
    max-width: 95vw;
  }

  .export-badge {
    background-color: #3b99fc;
    color: #ffffff;
    font-size: 0.65rem;
    font-weight: 800;
    padding: 2px 6px;
    border-radius: 3px;
    margin-right: 6px;
  }

  .export-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    background-color: #121316;
    border: 1px solid #22242a;
    border-radius: 6px;
  }

  .export-section-title {
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: #8a8f9d;
  }

  .export-options-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .export-radio-btn {
    display: flex;
    flex-direction: column;
    padding: 8px;
    background-color: #18191e;
    border: 1px solid #2d2f38;
    border-radius: 5px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .export-radio-btn input {
    display: none;
  }

  .export-radio-btn.selected {
    background-color: rgba(59, 153, 252, 0.15);
    border-color: #3b99fc;
  }

  .export-radio-btn .radio-title {
    font-size: 0.75rem;
    font-weight: 700;
    color: #e2e8f0;
    margin-bottom: 2px;
  }

  .export-radio-btn.selected .radio-title {
    color: #60a5fa;
  }

  .export-radio-btn .radio-desc {
    font-size: 0.62rem;
    color: #858997;
    line-height: 1.2;
  }

  .export-range-row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .export-radio-pill {
    padding: 5px 10px;
    font-size: 0.72rem;
    font-weight: 600;
    background-color: #18191e;
    border: 1px solid #2d2f38;
    border-radius: 4px;
    color: #9499a8;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .export-radio-pill input {
    display: none;
  }

  .export-radio-pill.active {
    background-color: #3b99fc;
    border-color: #3b99fc;
    color: #ffffff;
  }

  .export-radio-pill.disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .export-select {
    width: 100%;
    background-color: #18191e;
    border: 1px solid #2d2f38;
    color: #e2e8f0;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 0.75rem;
    margin-top: 4px;
  }

  .export-checkboxes-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 8px;
  }

  .export-check-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 0.75rem;
    color: #d1d5db;
    cursor: pointer;
    background-color: #18191e;
    padding: 6px 8px;
    border-radius: 4px;
    border: 1px solid #24262f;
  }

  .export-check-item input[type="checkbox"] {
    margin-top: 2px;
    accent-color: #3b99fc;
  }

  .check-label {
    display: flex;
    flex-direction: column;
  }

  .check-sub {
    font-size: 0.62rem;
    color: #717684;
  }

  .export-feedback-msg {
    padding: 8px 12px;
    border-radius: 4px;
    font-size: 0.75rem;
    font-weight: 600;
    text-align: center;
    white-space: pre-line;
  }

  .export-success {
    background-color: rgba(48, 209, 88, 0.15);
    border: 1px solid rgba(48, 209, 88, 0.4);
    color: #30d158;
  }

  .export-error {
    background-color: rgba(255, 69, 58, 0.15);
    border: 1px solid rgba(255, 69, 58, 0.4);
    color: #ff453a;
  }

  .cancel-btn {
    background-color: #27272a !important;
    border: 1px solid #3f3f46 !important;
    color: #a1a1aa !important;
  }

  .cancel-btn:hover:not(:disabled) {
    background-color: #3f3f46 !important;
    color: #ffffff !important;
  }

  .export-run-btn {
    background: linear-gradient(180deg, #007aff 0%, #0060df 100%) !important;
  }

  /* Show Control & Remotes Modal Styles */
  .remote-modal-card {
    width: 600px;
    max-width: 95vw;
  }

  .remote-badge {
    background-color: #af52de;
    color: #ffffff;
    font-size: 0.65rem;
    font-weight: 800;
    padding: 2px 6px;
    border-radius: 3px;
    margin-right: 6px;
  }

  .remote-section-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }

  .remote-status-badge {
    font-size: 0.62rem;
    font-weight: 800;
    letter-spacing: 0.5px;
  }

  .running-badge {
    color: #30d158;
  }

  .remote-info-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background-color: #0d0e11;
    border: 1px solid #1e2026;
    border-radius: 5px;
    padding: 8px 10px;
  }

  .remote-endpoint-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .endpoint-label {
    font-size: 0.7rem;
    color: #9499a8;
  }

  .endpoint-val {
    font-family: Menlo, monospace;
    font-size: 0.72rem;
    background-color: #1a1b22;
    border: 1px solid #2e313d;
    padding: 2px 6px;
    border-radius: 3px;
    color: #64d2ff;
  }

  .remote-desc-text {
    font-size: 0.68rem;
    color: #7b8090;
    margin: 0;
    line-height: 1.3;
  }

  .quick-commands-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }

  .cmd-pill {
    background-color: #17181e;
    border: 1px solid #282a35;
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.65rem;
    color: #cfd3dc;
  }

  .cmd-pill code {
    color: #3b99fc;
    font-weight: 600;
  }

  .osc-path-examples {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .osc-path {
    background-color: #17181e;
    border: 1px solid #282a35;
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.65rem;
    color: #30d158;
  }

  .mini-refresh-btn {
    background: transparent;
    border: 1px solid #383840;
    color: #a1a1aa;
    font-size: 0.65rem;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .mini-refresh-btn:hover {
    background-color: #27272e;
    color: #ffffff;
  }

  .midi-connect-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .connect-btn {
    white-space: nowrap;
    padding: 6px 12px;
  }

  .midi-status-feedback {
    font-size: 0.68rem;
    font-weight: 600;
    color: #30d158;
    margin-top: 2px;
  }

  .midi-note-map-hint {
    font-size: 0.64rem;
    color: #7b8090;
    line-height: 1.3;
    margin-top: 4px;
  }
</style>
