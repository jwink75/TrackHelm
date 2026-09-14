<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import * as pdfjsLib from "pdfjs-dist";
  import pdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

  import { 
    parseSetlistCsv, 
    resolveSetlistLocal, 
    findBestMatch, 
    calculateMatchScore, 
    normalizeSongTitle, 
    type ScannedAsset, 
    type ResolvedSetlistItem 
  } from "./lib/setlistResolver";
  import { analyzePlaylistHealth, type PlaylistItemHealth } from "./lib/playlistRepair";

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
  let cloudFolders: Array<{ name: string; path: string; is_dir: boolean; size_bytes: number; kind: string }> = [];
  let systemDrives: Array<{ name: string; path: string; is_dir: boolean; size_bytes: number; kind: string }> = [];
  let rootName = "This PC";
  let homePath = "";
  let showCloudDropdown = false;

  $: isCurrentPathHome = Boolean(homePath && (currentPath === homePath || currentPath === "~"));
  $: isCurrentPathCloud = Boolean(cloudFolders && cloudFolders.some(c => 
    c.path && currentPath.toLowerCase().startsWith(c.path.toLowerCase())
  ));

  function getFolderDisplayName(p: string): string {
    if (!p) return "";
    if (p === rootName) return `💻 ${rootName}`;
    const drive = systemDrives.find(d => 
      d.path.toUpperCase() === p.toUpperCase() || 
      (d.path.toUpperCase() + "\\") === p.toUpperCase() ||
      d.path.toUpperCase() === (p.toUpperCase() + "\\")
    );
    if (drive) return `💾 ${drive.name}`;
    const cloud = cloudFolders.find(c => c.path.toUpperCase() === p.toUpperCase());
    if (cloud) return `☁️ ${cloud.name}`;
    const normalized = p.replace(/\\/g, "/").replace(/\/$/, "");
    const parts = normalized.split("/");
    return parts.pop() || normalized || p;
  }

  function handleCloudJumpClick(e: MouseEvent) {
    e.stopPropagation();
    if (!cloudFolders || cloudFolders.length === 0) return;
    if (cloudFolders.length === 1) {
      loadBrowser(cloudFolders[0].path);
    } else {
      showCloudDropdown = !showCloudDropdown;
    }
  }
  
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
  let showTypeaheadIndicator = false;
  let lastFocusedPane: "sidebar" | "waveform" = "sidebar";
  
  // Browser Multi-selection
  let selectedFilePaths = new Set<string>();
  let lastSelectedEntry: { name: string; path: string } | null = null;

  // Custom Context Menu State
  let showContextMenu = false;
  let contextMenuType: "browser" | "waveform" | "assoc-file" = "browser";
  let contextMenuX = 0;
  let contextMenuY = 0;
  let contextMenuTargetFile: { name: string; path: string } | null = null;
  let contextMenuTargetAssoc: AssociatedFileItem | null = null;
  let preferHighResAudio: boolean = localStorage.getItem("th_prefer_high_res") === "true";
  let primarySongTrackPath: string | null = null;
  let abPreviousTrackPath: string | null = null;

  // Playlist State (saved in localStorage)
  interface PlaylistItem {
    name: string;
    path: string;
    isPlaceholder?: boolean;
    missingAudio?: boolean;
    alternatePath?: string;
    pdfPath?: string;
    singers?: string;
    keyNote?: string;
    notesMarkdown?: string;
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
    role?: string;
  }
  let associatedFiles: AssociatedFileItem[] = [];

  // Reactive unified song files list including active filePath so its row and stem actions are always accessible
  $: allSongFiles = (() => {
    const list: AssociatedFileItem[] = [];
    const seenPaths = new Set<string>();

    // If we have an active track loaded, make sure it is in the list
    if (filePath) {
      const existing = associatedFiles.find(f => f.path === filePath);
      if (existing) {
        list.push({ ...existing });
        seenPaths.add(filePath);
      } else {
        const isPrimary = filePath === primarySongTrackPath || !primarySongTrackPath;
        const isOrig = filePath.toLowerCase().includes("original") || (fileName && fileName.toLowerCase().includes("original"));
        const isVoc = filePath.toLowerCase().includes("vocals") || (fileName && fileName.toLowerCase().includes("vocals"));
        const isLead = filePath.toLowerCase().includes("lead") || (fileName && fileName.toLowerCase().includes("lead"));
        const isBacking = filePath.toLowerCase().includes("backing") || (fileName && fileName.toLowerCase().includes("backing"));
        const role = isLead ? "lead" : isBacking ? "backings" : isVoc ? "vocals" : isOrig ? "orig" : isLosslessAudio(filePath) ? "hires" : isPrimary ? "main" : "assoc";

        list.push({
          id: "active-track-" + filePath.replace(/[^a-zA-Z0-9]/g, "_"),
          name: fileName || filePath.split("/").pop() || filePath,
          path: filePath,
          fileType: "audio",
          role
        });
        seenPaths.add(filePath);
      }
    }

    // Append remaining associated files
    for (const f of associatedFiles) {
      if (f.path && !seenPaths.has(f.path)) {
        list.push(f);
        seenPaths.add(f.path);
      }
    }

    return list;
  })();

  // UVR Stem Separation state
  let uvrSeparating: boolean = false;
  let uvrStage: string = "";
  let uvrPercent: number = 0;
  let uvrCurrentTarget: string = "";
  let uvrErrorMessage: string | null = null;
  let uvrSuccessMessage: string | null = null;

  // Trash Deletion Confirmation Modal state
  let showTrashModal: boolean = false;
  let itemToTrash: AssociatedFileItem | null = null;
  let trashLoading: boolean = false;
  let trashErrorMessage: string | null = null;


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

  // Resizable Sidebars
  let leftSidebarWidth: number = 240;
  let rightSidebarWidth: number = 280;
  let isResizingLeft: boolean = false;
  let isResizingRight: boolean = false;

  function startResizeLeft(e: MouseEvent) {
    e.preventDefault();
    isResizingLeft = true;
    const startX = e.clientX;
    const initialWidth = leftSidebarWidth;
    let rafId: number | null = null;
    let latestClientX = startX;

    function onMouseMove(moveEvent: MouseEvent) {
      latestClientX = moveEvent.clientX;
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        const delta = latestClientX - startX;
        leftSidebarWidth = Math.max(160, Math.min(600, initialWidth + delta));
      });
    }

    function onMouseUp() {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      isResizingLeft = false;
      try {
        localStorage.setItem("th_left_sidebar_width", leftSidebarWidth.toString());
      } catch {}
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    }

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }

  function startResizeRight(e: MouseEvent) {
    e.preventDefault();
    isResizingRight = true;
    const startX = e.clientX;
    const initialWidth = rightSidebarWidth;
    let rafId: number | null = null;
    let latestClientX = startX;

    function onMouseMove(moveEvent: MouseEvent) {
      latestClientX = moveEvent.clientX;
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        const delta = startX - latestClientX;
        rightSidebarWidth = Math.max(180, Math.min(700, initialWidth + delta));
      });
    }

    function onMouseUp() {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      isResizingRight = false;
      try {
        localStorage.setItem("th_right_sidebar_width", rightSidebarWidth.toString());
      } catch {}
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    }

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }

  // Resizable Setlist Import Modal
  let setlistModalWidth: number = 960;
  let setlistModalHeight: number = 720;
  let isResizingSetlistModal: boolean = false;
  let isSetlistModalMaximized: boolean = false;
  let prevSetlistModalWidth: number = 960;
  let prevSetlistModalHeight: number = 720;

  function toggleMaximizeSetlistModal() {
    if (isSetlistModalMaximized) {
      setlistModalWidth = prevSetlistModalWidth;
      setlistModalHeight = prevSetlistModalHeight;
      isSetlistModalMaximized = false;
    } else {
      prevSetlistModalWidth = setlistModalWidth;
      prevSetlistModalHeight = setlistModalHeight;
      setlistModalWidth = Math.round(window.innerWidth * 0.95);
      setlistModalHeight = Math.round(window.innerHeight * 0.92);
      isSetlistModalMaximized = true;
    }
    try {
      localStorage.setItem("th_setlist_modal_w", setlistModalWidth.toString());
      localStorage.setItem("th_setlist_modal_h", setlistModalHeight.toString());
    } catch {}
  }

  function startResizeSetlistModal(e: MouseEvent) {
    e.preventDefault();
    isResizingSetlistModal = true;
    isSetlistModalMaximized = false;
    const startX = e.clientX;
    const startY = e.clientY;
    const initialW = setlistModalWidth;
    const initialH = setlistModalHeight;
    let rafId: number | null = null;
    let latestMeX = startX;
    let latestMeY = startY;

    function onMouseMove(me: MouseEvent) {
      latestMeX = me.clientX;
      latestMeY = me.clientY;
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        const deltaX = latestMeX - startX;
        const deltaY = latestMeY - startY;
        setlistModalWidth = Math.max(550, Math.min(window.innerWidth * 0.96, initialW + deltaX));
        setlistModalHeight = Math.max(400, Math.min(window.innerHeight * 0.94, initialH + deltaY));
      });
    }

    function onMouseUp() {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      isResizingSetlistModal = false;
      try {
        localStorage.setItem("th_setlist_modal_w", setlistModalWidth.toString());
        localStorage.setItem("th_setlist_modal_h", setlistModalHeight.toString());
      } catch {}
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    }

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }

  function getFileExtension(pathOrName: string): string {
    if (!pathOrName) return "";
    const clean = pathOrName.split("?")[0].split("#")[0];
    const dotIdx = clean.lastIndexOf(".");
    return dotIdx !== -1 ? clean.substring(dotIdx + 1).toUpperCase() : "";
  }

  function isLosslessAudio(pathOrName: string): boolean {
    const ext = getFileExtension(pathOrName).toLowerCase();
    return ["wav", "aiff", "aif", "flac", "alac"].includes(ext);
  }

  function isAudioFile(pathOrName: string): boolean {
    const ext = getFileExtension(pathOrName).toLowerCase();
    return ["wav", "mp3", "m4a", "aac", "flac", "aif", "aiff", "alac", "ogg", "wma"].includes(ext);
  }

  function renderMarkdown(md: string): string {
    if (!md || !md.trim()) return "<div class='markdown-empty-hint'>No notes recorded yet. Click <strong>Raw Text</strong> or <strong>Split</strong> to add rehearsal notes, singer tables, or chord cues...</div>";
    
    // First parse and convert Markdown tables
    const rawLines = md.split("\n");
    let inTable = false;
    let tableHtml = "";
    const processedLines: string[] = [];

    for (let i = 0; i < rawLines.length; i++) {
      const line = rawLines[i].trim();
      if (line.startsWith("|") && line.endsWith("|")) {
        const cells = line.slice(1, -1).split("|").map(c => c.trim());
        // Check if delimiter row e.g. |---|---| or |:---:|---:|
        if (cells.every(c => /^:?-+:?$/.test(c))) {
          continue;
        }
        if (!inTable) {
          inTable = true;
          tableHtml = '<div class="md-table-wrapper"><table class="md-table"><thead><tr>' + cells.map(c => `<th>${c}</th>`).join('') + '</tr></thead><tbody>';
        } else {
          tableHtml += '<tr>' + cells.map(c => `<td>${c}</td>`).join('') + '</tr>';
        }
      } else {
        if (inTable) {
          inTable = false;
          tableHtml += '</tbody></table></div>';
          processedLines.push(tableHtml);
          tableHtml = "";
        }
        processedLines.push(rawLines[i]);
      }
    }
    if (inTable) {
      tableHtml += '</tbody></table></div>';
      processedLines.push(tableHtml);
    }

    let html = processedLines.join("\n")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

    // Restore table tags that were HTML-escaped
    html = html
      .replace(/&lt;div class="md-table-wrapper"&gt;&lt;table class="md-table"&gt;&lt;thead&gt;&lt;tr&gt;/g, '<div class="md-table-wrapper"><table class="md-table"><thead><tr>')
      .replace(/&lt;\/tr&gt;&lt;\/thead&gt;&lt;tbody&gt;/g, '</tr></thead><tbody>')
      .replace(/&lt;\/tbody&gt;&lt;\/table&gt;&lt;\/div&gt;/g, '</tbody></table></div>')
      .replace(/&lt;th&gt;/g, '<th>')
      .replace(/&lt;\/th&gt;/g, '</th>')
      .replace(/&lt;tr&gt;/g, '<tr>')
      .replace(/&lt;\/tr&gt;/g, '</tr>')
      .replace(/&lt;td&gt;/g, '<td>')
      .replace(/&lt;\/td&gt;/g, '</td>');

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

  // Multi-Node EQ State (Precision Parametric EQ)
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
    if (!nodes || nodes.length === 0 || isEqBypassed) return "M 20 120 L 580 120";
    const activeNodes = nodes.filter(n => n.enabled);
    if (activeNodes.length === 0) return "M 20 120 L 580 120";

    const points: string[] = [];
    const numPoints = 140;
    for (let i = 0; i <= numPoints; i++) {
      const norm = i / numPoints;
      const freq = 20.0 * Math.pow(1000.0, norm); // 20Hz to 20,000Hz log scale
      const x = 20 + norm * 560;

      // Calculate analytical magnitude response for cascaded RBJ filters
      let totalGainDb = 0.0;
      for (const node of activeNodes) {
        const f0 = Math.max(20.0, Math.min(20000.0, node.freq));
        const g = node.gainDb;
        const q = Math.max(0.1, node.q);
        const fRatio = freq / f0;
        const logRatio = Math.log2(fRatio);

        if (node.filterType === "Peaking") {
          // Standard parametric bell curve: Gaussian in octave space scaled by Q
          const bandWidthOct = 1.0 / q;
          const bell = Math.exp(-0.5 * Math.pow(logRatio / (bandWidthOct * 0.5), 2));
          totalGainDb += g * bell;
        } else if (node.filterType === "LowShelf") {
          // Low shelf transition response
          const shelf = 1.0 / (1.0 + Math.pow(fRatio, 2.0 * Math.max(0.5, q)));
          totalGainDb += g * shelf;
        } else if (node.filterType === "HighShelf") {
          // High shelf transition response
          const shelf = 1.0 / (1.0 + Math.pow(1.0 / fRatio, 2.0 * Math.max(0.5, q)));
          totalGainDb += g * shelf;
        } else if (node.filterType === "LowPass") {
          if (freq > f0) {
            totalGainDb -= Math.min(48.0, 12.0 * Math.log2(freq / f0) * Math.max(0.707, q));
          }
        } else if (node.filterType === "HighPass") {
          if (freq < f0) {
            totalGainDb -= Math.min(48.0, 12.0 * Math.log2(f0 / freq) * Math.max(0.707, q));
          }
        } else if (node.filterType === "Notch") {
          const dist = Math.abs(logRatio);
          const width = 0.15 / q;
          if (dist < width) {
            totalGainDb -= Math.min(36.0, (1.0 - dist / width) * 36.0);
          }
        } else if (node.filterType === "BandPass") {
          const dist = Math.abs(logRatio);
          const width = 0.2 / q;
          if (dist > width) {
            totalGainDb -= Math.min(36.0, (dist - width) * 18.0);
          }
        }
      }

      const clampedGain = Math.max(-24.0, Math.min(24.0, totalGainDb));
      const y = 120 - clampedGain * (100.0 / 24.0);
      points.push(`${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`);
    }
    return points.join(" ");
  }

  // Real-Time Animated "Spiky" Frequency Spectrum Analyzer (RTA)
  function getLiveRtaSpectrumPath(playing: boolean, inDb: number, timeSec: number): string {
    if (!playing || inDb <= -58.0) {
      return "M 20 220 L 580 220 Z";
    }

    const numBars = 64;
    const baseEnergy = Math.max(0.05, Math.min(1.0, (inDb + 60.0) / 60.0));
    let path = "M 20 220";

    for (let i = 0; i <= numBars; i++) {
      const norm = i / numBars;
      const x = 20 + norm * 560;

      // Realistic musical spectrum shape: heavier low/mids with harmonic peaks
      const pinkDrop = Math.pow(1.0 - norm * 0.45, 1.8);
      
      // Spiky harmonic oscillations based on playback time & band index
      const h1 = Math.abs(Math.sin((i * 0.42) + timeSec * 14.0));
      const h2 = Math.abs(Math.cos((i * 0.85) - timeSec * 9.0));
      const h3 = Math.abs(Math.sin((i * 1.7) + timeSec * 22.0));
      const spikeFactor = 0.35 + 0.35 * h1 + 0.2 * h2 + 0.1 * h3;

      // Add transient spikes around drum/vocal resonance regions (e.g. index 8-16 bass, 24-36 mids, 44-52 presence)
      let resonance = 1.0;
      if (i >= 6 && i <= 14) resonance += 0.4 * Math.sin(timeSec * 16.0);
      if (i >= 22 && i <= 34) resonance += 0.35 * Math.cos(timeSec * 20.0);
      if (i >= 42 && i <= 50) resonance += 0.25 * Math.sin(timeSec * 26.0);

      const heightPx = Math.min(180, Math.max(4, baseEnergy * pinkDrop * spikeFactor * resonance * 175));
      const y = 220 - heightPx;
      path += ` L ${x.toFixed(1)} ${y.toFixed(1)}`;
    }

    path += " L 580 220 Z";
    return path;
  }

  function isCoreEqNode(nodeId: string): boolean {
    return nodeId === "node-1" || nodeId === "node-2" || nodeId === "node-3";
  }

  $: sortedEqNodes = [...eqNodes].sort((a, b) => a.freq - b.freq);

  // Interactive EQ Node Dragging & Mousewheel State
  let isDraggingEqNode = false;
  let draggedEqNodeId: string | null = null;
  let isDraggingEqWidth = false;
  let draggedEqWidthNodeId: string | null = null;
  let draggedEqWidthSide: "left" | "right" = "right";
  let eqSvgElement: SVGSVGElement | null = null;

  function handleEqNodeMousedown(e: MouseEvent, nodeId: string) {
    if (e.button !== 0) return; // Left click only
    e.stopPropagation();
    isDraggingEqNode = true;
    draggedEqNodeId = nodeId;
    selectedEqNodeId = nodeId;
  }

  function handleEqWidthHandleMousedown(e: MouseEvent, nodeId: string, side: "left" | "right") {
    if (e.button !== 0) return;
    e.stopPropagation();
    isDraggingEqWidth = true;
    draggedEqWidthNodeId = nodeId;
    draggedEqWidthSide = side;
    selectedEqNodeId = nodeId;
  }

  function handleEqSvgMousedown(e: MouseEvent) {
    if (e.button !== 0) return;
    if (!eqSvgElement) return;
    const rect = eqSvgElement.getBoundingClientRect();
    const scaleX = 600 / rect.width;
    const scaleY = 240 / rect.height;
    const svgX = (e.clientX - rect.left) * scaleX;
    const svgY = (e.clientY - rect.top) * scaleY;

    if (svgX >= 20 && svgX <= 580 && svgY >= 20 && svgY <= 220) {
      const clickedNode = eqNodes.find(n => {
        const nx = 20 + ((Math.log10(Math.max(20, Math.min(20000, n.freq))) - 1.30103) / 3.0) * 560;
        const ny = n.enabled ? 120 - Math.max(-24, Math.min(24, n.gainDb)) * (100 / 24) : 120;
        return Math.hypot(svgX - nx, svgY - ny) < 22;
      });

      if (clickedNode) {
        selectedEqNodeId = clickedNode.id;
        isDraggingEqNode = true;
        draggedEqNodeId = clickedNode.id;
      }
    }
  }

  function handleEqSvgDblClick(e: MouseEvent) {
    if (!eqSvgElement) return;
    const rect = eqSvgElement.getBoundingClientRect();
    const scaleX = 600 / rect.width;
    const scaleY = 240 / rect.height;
    const svgX = (e.clientX - rect.left) * scaleX;
    const svgY = (e.clientY - rect.top) * scaleY;

    if (svgX >= 20 && svgX <= 580 && svgY >= 20 && svgY <= 220) {
      const normX = (svgX - 20) / 560.0;
      const freq = Math.round(Math.pow(10, 1.30103 + normX * 3.0));
      const gainDb = parseFloat(Math.max(-24, Math.min(24, (120 - svgY) / (100 / 24))).toFixed(1));

      const colors = ["#64d2ff", "#ff375f", "#ffd60a", "#30d158", "#bf5af2", "#ff9f0a"];
      const newId = "node-" + (eqNodes.length + 1);
      const newColor = colors[eqNodes.length % colors.length];

      eqNodes = [...eqNodes, {
        id: newId,
        name: "Band " + (eqNodes.length + 1),
        filterType: "Peaking",
        freq: Math.max(20, Math.min(20000, freq)),
        gainDb: gainDb,
        q: 1.0,
        enabled: true,
        color: newColor
      }];
      selectedEqNodeId = newId;
      isDraggingEqNode = true;
      draggedEqNodeId = newId;
      updateEqEngine();
    }
  }

  function handleEqSvgMousemove(e: MouseEvent) {
    if (!eqSvgElement) return;
    const rect = eqSvgElement.getBoundingClientRect();
    const scaleX = 600 / rect.width;
    const scaleY = 240 / rect.height;
    const svgX = (e.clientX - rect.left) * scaleX;
    const svgY = (e.clientY - rect.top) * scaleY;

    // Handle width / Q dragging
    if (isDraggingEqWidth && draggedEqWidthNodeId) {
      const node = eqNodes.find(n => n.id === draggedEqWidthNodeId);
      if (!node) return;
      const nx = 20 + ((Math.log10(Math.max(20, Math.min(20000, node.freq))) - 1.30103) / 3.0) * 560;
      const dist = Math.abs(svgX - nx);
      // Q is inversely proportional to bandwidth width: q = 60 / dist
      const newQ = parseFloat(Math.max(0.1, Math.min(10.0, 60.0 / Math.max(6.0, dist))).toFixed(2));
      node.q = newQ;
      eqNodes = [...eqNodes];
      updateEqEngine();
      return;
    }

    // Handle center node dragging (frequency & gain)
    if (isDraggingEqNode && draggedEqNodeId) {
      const node = eqNodes.find(n => n.id === draggedEqNodeId);
      if (!node) return;

      const clampedX = Math.max(20, Math.min(580, svgX));
      const clampedY = Math.max(20, Math.min(220, svgY));

      const normX = (clampedX - 20) / 560.0;
      const freq = Math.round(Math.pow(10, 1.30103 + normX * 3.0));
      node.freq = Math.max(20, Math.min(20000, freq));

      if (node.filterType === 'Peaking' || node.filterType === 'LowShelf' || node.filterType === 'HighShelf') {
        node.gainDb = parseFloat(Math.max(-24, Math.min(24, (120 - clampedY) / (100 / 24))).toFixed(1));
      }

      eqNodes = [...eqNodes];
      updateEqEngine();
    }
  }

  function handleEqSvgMouseup() {
    if (isDraggingEqNode) {
      isDraggingEqNode = false;
      draggedEqNodeId = null;
    }
    if (isDraggingEqWidth) {
      isDraggingEqWidth = false;
      draggedEqWidthNodeId = null;
    }
  }

  function handleEqSvgWheel(e: WheelEvent, node: EqNodeState) {
    e.preventDefault();
    const delta = e.deltaY < 0 ? 0.1 : -0.1;
    node.q = parseFloat(Math.max(0.1, Math.min(10.0, node.q + delta)).toFixed(2));
    eqNodes = [...eqNodes];
    updateEqEngine();
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

  function computeCompressorOutDb(stage: CompStageState, inDb: number): number {
    if (!stage) return inDb;
    const T = stage.thresholdDb;
    const R = Math.max(1.0, stage.ratio);
    const W = Math.max(0.0, stage.kneeDb);
    const makeup = stage.makeupDb || 0.0;

    let outDb = inDb;
    if (W > 0.1) {
      const halfW = W / 2.0;
      if (inDb <= T - halfW) {
        outDb = inDb;
      } else if (inDb > T + halfW) {
        outDb = T + (inDb - T) / R;
      } else {
        const diff = inDb - T + halfW;
        outDb = inDb + ((1.0 / R - 1.0) * diff * diff) / (2.0 * W);
      }
    } else {
      if (inDb <= T) {
        outDb = inDb;
      } else {
        outDb = T + (inDb - T) / R;
      }
    }
    return outDb + makeup;
  }

  function getSonitusCurvePath(stage: CompStageState): string {
    if (!stage) return "M 20 180 L 280 20";
    const numPoints = 80;
    let path = "";

    for (let i = 0; i <= numPoints; i++) {
      const inDb = -60.0 + (i / numPoints) * 60.0; // from -60 dB to 0 dB
      const outDb = computeCompressorOutDb(stage, inDb);

      const pxX = 20 + (60 + inDb) * (260 / 60);
      const pxY = 180 - (60 + Math.max(-60, Math.min(6, outDb))) * (160 / 60);

      if (i === 0) {
        path = `M ${pxX.toFixed(1)} ${pxY.toFixed(1)}`;
      } else {
        path += ` L ${pxX.toFixed(1)} ${pxY.toFixed(1)}`;
      }
    }
    return path;
  }

  function getSonitusSignalDot(stage: CompStageState, inDb: number): { x: number, y: number } {
    if (!stage) return { x: 20, y: 180 };
    const clampedIn = Math.max(-60, Math.min(0, inDb));
    const outDb = computeCompressorOutDb(stage, clampedIn);

    const liveDotX = 20 + (60 + clampedIn) * (260 / 60);
    const liveDotY = 180 - (60 + Math.max(-60, Math.min(6, outDb))) * (160 / 60);
    return { x: liveDotX, y: liveDotY };
  }

  // Volume Envelope State (+12dB Max, Multiple Curves, Pre-Dynamics)
  type EnvelopeCurve = "linear" | "curve_up" | "curve_down" | "sine";

  interface VolumeEnvelopeNode {
    id: string;
    timeSeconds: number;
    gainDb: number; // -60.0 to +12.0 dB
    curve: EnvelopeCurve;
  }

  let volumeEnvelopeNodes: VolumeEnvelopeNode[] = [];
  let isEnvelopeEnabled: boolean = true;
  let isEnvelopeOverlayVisible: boolean = true;
  let selectedEnvelopeNodeId: string | null = null;
  let hoveredEnvelopeNodeId: string | null = null;
  let isDraggingEnvelopeNode: boolean = false;
  let draggedEnvelopeNodeId: string | null = null;
  let exportBakeEnvelope: boolean = true;

  // Selective Migration Options for Upgraded Mixes
  let repairMigrateMarkers: boolean = true;
  let repairMigrateRegions: boolean = true;
  let repairMigrateNotes: boolean = true;
  let repairMigrateVolume: boolean = false;
  let repairMigrateEnvelope: boolean = false;
  let repairMigrateEq: boolean = false;
  let repairMigrateComp: boolean = false;

  function calcEnvelopeGainDbAt(tSec: number): number {
    if (!isEnvelopeEnabled || volumeEnvelopeNodes.length === 0) return 0.0;
    const sorted = [...volumeEnvelopeNodes].sort((a, b) => a.timeSeconds - b.timeSeconds);
    if (tSec <= sorted[0].timeSeconds) return sorted[0].gainDb;
    const last = sorted[sorted.length - 1];
    if (tSec >= last.timeSeconds) return last.gainDb;

    for (let i = 0; i < sorted.length - 1; i++) {
      const a = sorted[i];
      const b = sorted[i + 1];
      if (tSec >= a.timeSeconds && tSec <= b.timeSeconds) {
        const segLen = Math.max(1e-6, b.timeSeconds - a.timeSeconds);
        const normT = Math.max(0, Math.min(1, (tSec - a.timeSeconds) / segLen));
        let shapedT = normT;
        if (a.curve === "curve_up") shapedT = normT * normT;
        else if (a.curve === "curve_down") shapedT = 1.0 - (1.0 - normT) * (1.0 - normT);
        else if (a.curve === "sine") shapedT = 0.5 * (1.0 - Math.cos(Math.PI * normT));
        return a.gainDb + (b.gainDb - a.gainDb) * shapedT;
      }
    }
    return 0.0;
  }

  function calcEnvelopeLinearGainAt(tSec: number): number {
    const db = calcEnvelopeGainDbAt(tSec);
    if (db <= -59.5) return 0.0;
    return Math.pow(10, db / 20.0);
  }

  async function updateVolumeEnvelopeEngine() {
    const nodes = isEnvelopeEnabled ? [...volumeEnvelopeNodes].sort((a, b) => a.timeSeconds - b.timeSeconds) : [];
    await invoke("set_volume_envelope", {
      nodes: nodes.map(n => ({
        timeSeconds: n.timeSeconds,
        gainDb: n.gainDb,
        curve: n.curve
      }))
    });
    saveCurrentTrackProfile(filePath);
    drawMainWaveform();
  }

  function resetVolumeEnvelope() {
    volumeEnvelopeNodes = [];
    selectedEnvelopeNodeId = null;
    updateVolumeEnvelopeEngine();
  }

  function addVolumeEnvelopeNodeAt(timeSec: number, gainDb = 0.0, curve: EnvelopeCurve = "linear") {
    const clampedTime = Math.max(0, Math.min(duration || 0, timeSec));
    const clampedGain = Math.max(-60, Math.min(12, gainDb));
    const id = "env-" + Date.now() + "-" + Math.random().toString(36).substring(2, 5);
    volumeEnvelopeNodes = [...volumeEnvelopeNodes, {
      id,
      timeSeconds: clampedTime,
      gainDb: clampedGain,
      curve
    }].sort((a, b) => a.timeSeconds - b.timeSeconds);
    selectedEnvelopeNodeId = id;
    updateVolumeEnvelopeEngine();
  }

  function removeVolumeEnvelopeNode(id: string) {
    volumeEnvelopeNodes = volumeEnvelopeNodes.filter(n => n.id !== id);
    if (selectedEnvelopeNodeId === id) selectedEnvelopeNodeId = null;
    updateVolumeEnvelopeEngine();
  }

  function setEnvelopeNodeCurve(nodeId: string, curve: EnvelopeCurve) {
    const node = volumeEnvelopeNodes.find(n => n.id === nodeId);
    if (!node) return;
    node.curve = curve;
    volumeEnvelopeNodes = [...volumeEnvelopeNodes];
    updateVolumeEnvelopeEngine();
  }

  function updateEnvelopeNodeGain(nodeId: string, gainDb: number) {
    const node = volumeEnvelopeNodes.find(n => n.id === nodeId);
    if (!node) return;
    node.gainDb = Math.max(-60, Math.min(12, gainDb));
    volumeEnvelopeNodes = [...volumeEnvelopeNodes];
    updateVolumeEnvelopeEngine();
  }

  function cycleEnvelopeNodeCurve(nodeId: string) {
    const node = volumeEnvelopeNodes.find(n => n.id === nodeId);
    if (!node) return;
    const curves: EnvelopeCurve[] = ["linear", "curve_up", "curve_down", "sine"];
    const curIdx = curves.indexOf(node.curve);
    node.curve = curves[(curIdx + 1) % curves.length];
    volumeEnvelopeNodes = [...volumeEnvelopeNodes];
    updateVolumeEnvelopeEngine();
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

  // Application Preferences & Display Settings (Cmd+,)
  type PdfDefaultTheme = "light" | "dark" | "system";
  type AiProvider = "builtin" | "ollama" | "openai" | "anthropic" | "gemini";

  let prefPdfDefaultTheme: PdfDefaultTheme = "system";
  let prefFolderAac: string = "";
  let prefFolderHiRes: string = "";
  let prefFolderOrig: string = "";
  let prefFolderPdf: string = "";
  let prefFolderVocals: string = "";
  let prefAiProvider: AiProvider = "builtin";
  let prefAiApiKey: string = "";
  let prefAiModel: string = "";
  let prefOllamaUrl: string = "http://localhost:11434";
  let showPreferencesModal: boolean = false;
  let showAboutModal: boolean = false;

  let isAutoLinkingLibrary: boolean = false;
  let autoLinkStatusMessage: string = "";
  let autoLinkStatusIsError: boolean = false;

  // Setlist CSV Importer Modal State
  let showSetlistModal: boolean = false;
  let rawCsvText: string = "";
  let isResolvingSetlist: boolean = false;
  let resolvedSetlist: ResolvedSetlistItem[] = [];
  let setlistResolveError: string = "";

  // Playlist Repair & Missing Files State
  let showRepairModal: boolean = false;
  let isScanningHealth: boolean = false;
  let playlistHealth: PlaylistItemHealth[] = [];
  let missingTracksCount: number = 0;

  function loadAppPreferences() {
    try {
      const savedTheme = localStorage.getItem("th_pref_pdf_theme");
      if (savedTheme === "light" || savedTheme === "dark" || savedTheme === "system") {
        prefPdfDefaultTheme = savedTheme;
      }
      prefFolderAac = localStorage.getItem("th_pref_folder_aac") || "";
      prefFolderHiRes = localStorage.getItem("th_pref_folder_hires") || "";
      prefFolderOrig = localStorage.getItem("th_pref_folder_orig") || "";
      prefFolderPdf = localStorage.getItem("th_pref_folder_pdf") || "";
      prefFolderVocals = localStorage.getItem("th_pref_folder_vocals") || "";

      const savedAi = localStorage.getItem("th_pref_ai_provider");
      if (savedAi === "builtin" || savedAi === "ollama" || savedAi === "openai" || savedAi === "anthropic" || savedAi === "gemini") {
        prefAiProvider = savedAi;
      }
      prefAiApiKey = localStorage.getItem("th_pref_ai_api_key") || "";
      prefAiModel = localStorage.getItem("th_pref_ai_model") || "";
      prefOllamaUrl = localStorage.getItem("th_pref_ollama_url") || "http://localhost:11434";
    } catch (e) {}
  }

  function saveAppPreferences() {
    try {
      localStorage.setItem("th_pref_pdf_theme", prefPdfDefaultTheme);
      localStorage.setItem("th_pref_folder_aac", prefFolderAac);
      localStorage.setItem("th_pref_folder_hires", prefFolderHiRes);
      localStorage.setItem("th_pref_folder_orig", prefFolderOrig);
      localStorage.setItem("th_pref_folder_pdf", prefFolderPdf);
      localStorage.setItem("th_pref_folder_vocals", prefFolderVocals);
      localStorage.setItem("th_pref_ai_provider", prefAiProvider);
      localStorage.setItem("th_pref_ai_api_key", prefAiApiKey);
      localStorage.setItem("th_pref_ai_model", prefAiModel);
      localStorage.setItem("th_pref_ollama_url", prefOllamaUrl);
    } catch (e) {}
  }

  function setPdfDefaultTheme(theme: PdfDefaultTheme) {
    prefPdfDefaultTheme = theme;
    saveAppPreferences();
  }

  async function pickPreferenceFolder(key: "aac" | "hires" | "orig" | "pdf" | "vocals") {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Rehearsal Assets Folder"
      });
      if (selected && typeof selected === "string") {
        if (key === "aac") prefFolderAac = selected;
        else if (key === "hires") prefFolderHiRes = selected;
        else if (key === "orig") prefFolderOrig = selected;
        else if (key === "pdf") prefFolderPdf = selected;
        else if (key === "vocals") prefFolderVocals = selected;
        saveAppPreferences();
      }
    } catch (err) {
      console.error("Failed to select folder:", err);
    }
  }

  function shouldInvertPdfByDefault(): boolean {
    if (prefPdfDefaultTheme === "dark") return true;
    if (prefPdfDefaultTheme === "light") return false;
    // "system": Follow OS dark mode
    if (typeof window !== "undefined" && window.matchMedia) {
      return window.matchMedia("(prefers-color-scheme: dark)").matches;
    }
    return false;
  }

  function openPreferencesModal() {
    showPreferencesModal = true;
  }

  function closePreferencesModal() {
    showPreferencesModal = false;
  }

  function openAboutModal() {
    showAboutModal = true;
  }

  function closeAboutModal() {
    showAboutModal = false;
  }

  async function scanLibraryAssets() {
    let aacFiles: ScannedAsset[] = [];
    let hiresFiles: ScannedAsset[] = [];
    let origFiles: ScannedAsset[] = [];
    let pdfFiles: ScannedAsset[] = [];
    let vocalsFiles: ScannedAsset[] = [];

    try {
      if (prefFolderAac) {
        aacFiles = await invoke("scan_library_folder", {
          folderPath: prefFolderAac,
          extensions: ["m4a", "aac", "mp3", "wav", "aiff", "aif"]
        });
      }
      if (prefFolderHiRes) {
        hiresFiles = await invoke("scan_library_folder", {
          folderPath: prefFolderHiRes,
          extensions: ["wav", "flac", "aiff", "aif"]
        });
      }
      if (prefFolderOrig) {
        origFiles = await invoke("scan_library_folder", {
          folderPath: prefFolderOrig,
          extensions: ["mp3", "m4a", "wav", "flac", "aiff", "aif", "ogg"]
        });
      }
      if (prefFolderPdf) {
        pdfFiles = await invoke("scan_library_folder", {
          folderPath: prefFolderPdf,
          extensions: ["pdf"]
        });
      }
      if (prefFolderVocals) {
        vocalsFiles = await invoke("scan_library_folder", {
          folderPath: prefFolderVocals,
          extensions: ["m4a", "aac", "mp3", "wav", "flac", "aiff", "aif"]
        });
      }
    } catch (e) {
      console.error("Failed to scan library assets:", e);
    }

    return { aacFiles, hiresFiles, origFiles, pdfFiles, vocalsFiles };
  }

  async function executeAutoLinkLibrary() {
    if (isAutoLinkingLibrary) return;
    isAutoLinkingLibrary = true;
    autoLinkStatusMessage = "";
    autoLinkStatusIsError = false;

    try {
      // 1. Scan assets across all designated folders
      const { aacFiles, hiresFiles, origFiles, pdfFiles, vocalsFiles } = await scanLibraryAssets();

      const totalScanned = aacFiles.length + hiresFiles.length + origFiles.length + pdfFiles.length + (vocalsFiles ? vocalsFiles.length : 0);
      if (totalScanned === 0) {
        autoLinkStatusMessage = "No library files found. Please set your folder paths above first.";
        autoLinkStatusIsError = true;
        isAutoLinkingLibrary = false;
        return;
      }

      // Stem identifier helper
      const isStemOrVocal = (asset: ScannedAsset): boolean => {
        const pathLower = (asset.path || "").toLowerCase();
        const nameLower = (asset.name || "").toLowerCase();
        if (
          pathLower.includes("/vocals only/") || 
          pathLower.includes("\\vocals only\\") || 
          pathLower.includes("/stems/") || 
          pathLower.includes("\\stems\\")
        ) {
          return true;
        }
        return (
          nameLower.includes("(vocals ensemble)") ||
          nameLower.includes("(vocals)") ||
          nameLower.includes("(lead vocals)") ||
          nameLower.includes("(backing vocals)") ||
          nameLower.includes("(iso track)") ||
          nameLower.includes("acapella") ||
          nameLower.includes("isolated vocals") ||
          nameLower.includes("vocals only")
        );
      };

      // Collect stem pool (designated folder + any subfolder stems found)
      const stemPool: ScannedAsset[] = [...(vocalsFiles || [])];
      for (const f of [...aacFiles, ...hiresFiles, ...origFiles]) {
        if (isStemOrVocal(f) && !stemPool.some(s => s.path === f.path)) {
          stemPool.push(f);
        }
      }

      // Candidate anchor tracks (primary backing/rehearsal tracks from AAC and Hi-Res)
      const anchorMap = new Map<string, ScannedAsset>();
      for (const f of aacFiles) {
        if (!isStemOrVocal(f)) {
          anchorMap.set(f.path, f);
        }
      }
      for (const f of hiresFiles) {
        if (!isStemOrVocal(f)) {
          const norm = normalizeSongTitle(f.name);
          const alreadyInAac = Array.from(anchorMap.values()).some(a => normalizeSongTitle(a.name) === norm);
          if (!alreadyInAac) {
            anchorMap.set(f.path, f);
          }
        }
      }

      const anchors = Array.from(anchorMap.values());
      if (anchors.length === 0) {
        autoLinkStatusMessage = "No performance tracks found in AAC or Full-Res folders to anchor song collections.";
        autoLinkStatusIsError = true;
        isAutoLinkingLibrary = false;
        return;
      }

      const pdfPool = pdfFiles;
      const origPool = origFiles.filter(f => !isStemOrVocal(f));
      const hiresPool = hiresFiles.filter(f => !isStemOrVocal(f));

      const store = getProfilesStore();
      let songsProcessed = 0;
      let pdfsLinked = 0;
      let origsLinked = 0;
      let hiresLinked = 0;
      let stemsLinked = 0;

      for (const anchor of anchors) {
        songsProcessed++;
        const cleanTitle = normalizeSongTitle(anchor.name);
        if (!cleanTitle) continue;

        const existingProfile = store[anchor.path] || ({} as TrackProfile);
        const existingAssoc: AssociatedFileItem[] = existingProfile.associatedFiles ? [...existingProfile.associatedFiles] : [];

        // 1. Match Sheet Music PDF
        let matchedPdfPath = existingProfile.pdfChartPath || "";
        let matchedPdfName = existingProfile.pdfChartName || "";
        if (!matchedPdfPath && pdfPool.length > 0) {
          const match = findBestMatch(cleanTitle, pdfPool, 0.55);
          if (match) {
            matchedPdfPath = match.asset.path;
            matchedPdfName = match.asset.name;
          }
        }
        if (matchedPdfPath && !existingAssoc.some(a => a.path === matchedPdfPath)) {
          existingAssoc.push({
            id: "pdf_" + Math.random().toString(36).substring(2, 8),
            name: matchedPdfName || matchedPdfPath.split("/").pop() || "Sheet Music",
            path: matchedPdfPath,
            fileType: "pdf"
          });
          pdfsLinked++;
        }

        // 2. Match Original Artist Recording
        if (origPool.length > 0 && !existingAssoc.some(a => a.role === "orig" || a.id === "audio-orig")) {
          const match = findBestMatch(cleanTitle, origPool, 0.60);
          if (match && match.asset.path !== anchor.path) {
            existingAssoc.push({
              id: "audio-orig",
              name: match.asset.name,
              path: match.asset.path,
              fileType: "audio",
              role: "orig"
            });
            origsLinked++;
          }
        }

        // 3. Match Hi-Res Audio Track
        if (hiresPool.length > 0 && anchor.path !== hiresPool.find(h => h.path === anchor.path)?.path) {
          if (!existingAssoc.some(a => a.role === "hires" || a.id === "audio-hires")) {
            const match = findBestMatch(cleanTitle, hiresPool, 0.60);
            if (match && match.asset.path !== anchor.path) {
              existingAssoc.push({
                id: "audio-hires",
                name: match.asset.name,
                path: match.asset.path,
                fileType: "audio",
                role: "hires"
              });
              hiresLinked++;
            }
          }
        }

        // 4. Match All Vocal Stems & Iso Tracks
        if (stemPool.length > 0) {
          for (const stem of stemPool) {
            if (stem.path === anchor.path) continue;
            if (existingAssoc.some(a => a.path === stem.path)) continue;

            const score = calculateMatchScore(cleanTitle, stem.name);
            if (score >= 0.62) {
              const stemNameLower = stem.name.toLowerCase();
              let stemRole = "vocals";
              if (stemNameLower.includes("lead")) {
                stemRole = "lead";
              } else if (stemNameLower.includes("backing") || stemNameLower.includes("bgv") || stemNameLower.includes("backings")) {
                stemRole = "backings";
              } else if (stemNameLower.includes("iso track") || stemNameLower.includes("instrumental")) {
                stemRole = "track";
              }

              existingAssoc.push({
                id: "stem_" + Math.random().toString(36).substring(2, 8),
                name: stem.name,
                path: stem.path,
                fileType: "audio",
                role: stemRole
              });
              stemsLinked++;
            }
          }
        }

        // Update profile for this anchor track
        store[anchor.path] = {
          ...existingProfile,
          primarySongTrackPath: anchor.path,
          pdfChartPath: matchedPdfPath,
          pdfChartName: matchedPdfName,
          associatedFiles: existingAssoc
        };

        // Propagate bi-directional sync to all peer audio files
        const audioPeers = existingAssoc.filter(a => a.fileType === "audio" && a.path);
        for (const peer of audioPeers) {
          if (!peer.path || peer.path === anchor.path) continue;
          const peerExisting = store[peer.path] || ({} as TrackProfile);
          const peerAssoc: AssociatedFileItem[] = [
            ...existingAssoc.filter(a => a.fileType !== "audio"),
            {
              id: "track-anchor-" + anchor.path.replace(/[^a-zA-Z0-9]/g, "_"),
              name: anchor.name,
              path: anchor.path,
              fileType: "audio",
              role: isLosslessAudio(anchor.path) ? "hires" : "main"
            },
            ...audioPeers.filter(p => p.path !== peer.path)
          ];

          store[peer.path] = {
            ...peerExisting,
            primarySongTrackPath: anchor.path,
            pdfChartPath: matchedPdfPath || peerExisting.pdfChartPath || "",
            pdfChartName: matchedPdfName || peerExisting.pdfChartName || "",
            markers: existingProfile.markers ? existingProfile.markers.map(m => ({ ...m })) : (peerExisting.markers || []),
            notes: existingProfile.notes || peerExisting.notes || "",
            lyrics: existingProfile.lyrics || peerExisting.lyrics || "",
            associatedFiles: peerAssoc
          };
        }

        // If the currently loaded song in TrackHelm matches this anchor or any peer:
        if (filePath && (filePath === anchor.path || audioPeers.some(p => p.path === filePath))) {
          const currentCollectionProfile = store[filePath];
          if (currentCollectionProfile) {
            associatedFiles = [...(currentCollectionProfile.associatedFiles || [])];
            primarySongTrackPath = currentCollectionProfile.primarySongTrackPath || anchor.path;
            if (matchedPdfPath && !pdfChartPath) {
              pdfChartPath = matchedPdfPath;
              pdfChartName = matchedPdfName;
            }
          }
        }
      }

      flushProfilesToLocalStorage();
      autoLinkStatusMessage = `✓ Auto-link complete! Processed ${songsProcessed} songs. Linked ${pdfsLinked} PDFs, ${origsLinked} Originals, ${hiresLinked} Full-Res masters, and ${stemsLinked} Vocal/Iso stems.`;
      autoLinkStatusIsError = false;
    } catch (err: any) {
      console.error("Auto-link error:", err);
      autoLinkStatusMessage = "Failed to auto-link library: " + (err?.message || err);
      autoLinkStatusIsError = true;
    } finally {
      isAutoLinkingLibrary = false;
    }
  }

  async function openSetlistModal() {
    showSetlistModal = true;
    if (rawCsvText && resolvedSetlist.length === 0) {
      await handleResolveSetlist();
    }
  }

  async function loadSetlistCsvFile() {
    try {
      const selected = await open({
        multiple: false,
        title: "Select Setlist CSV / TSV / Text File",
        filters: [
          { name: "Delimited Text (*.csv, *.tsv, *.txt)", extensions: ["csv", "tsv", "txt"] }
        ]
      });
      if (!selected || typeof selected !== "string") return;

      const bytes: number[] = await invoke("read_file_bytes", { path: selected });
      const decoder = new TextDecoder("utf-8");
      const text = decoder.decode(new Uint8Array(bytes));
      if (text && text.trim()) {
        rawCsvText = text;
        await handleResolveSetlist();
      }
    } catch (err) {
      console.error("Failed to load setlist file:", err);
      setlistResolveError = "Failed to read file: " + err;
    }
  }

  async function handleResolveSetlist() {
    if (!rawCsvText.trim()) return;
    isResolvingSetlist = true;
    setlistResolveError = "";

    try {
      const rows = parseSetlistCsv(rawCsvText);
      if (rows.length === 0) {
        setlistResolveError = "No valid song rows detected in the pasted text.";
        isResolvingSetlist = false;
        return;
      }

      const libraries = await scanLibraryAssets();
      resolvedSetlist = resolveSetlistLocal(rows, libraries);
    } catch (err: any) {
      setlistResolveError = err?.message || String(err);
    } finally {
      isResolvingSetlist = false;
    }
  }

  function applyImportedSetlist() {
    if (resolvedSetlist.length === 0) return;

    const newItems: PlaylistItem[] = resolvedSetlist.map(item => ({
      name: item.title,
      path: item.mainAacPath || (item.isPlaceholder ? `[unlinked]:${item.title}` : ""),
      isPlaceholder: item.isPlaceholder,
      missingAudio: item.isPlaceholder,
      alternatePath: item.fullResWavPath || item.originalPath,
      pdfPath: item.pdfPath,
      singers: item.singers,
      keyNote: item.keyNote,
      notesMarkdown: item.notesMarkdown
    }));

    playlistItems = newItems;
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));

    // Save project profiles with notes and associated files for each song
    const store = getProfilesStore();
    resolvedSetlist.forEach(item => {
      const targetPath = item.mainAacPath || `[unlinked]:${item.title}`;
      const assoc: AssociatedFileItem[] = [];
      if (item.pdfPath && item.pdfName) {
        assoc.push({ id: "pdf-1", name: item.pdfName, path: item.pdfPath, fileType: "pdf" });
      }
      if (item.fullResWavPath && item.fullResWavName) {
        assoc.push({ id: "audio-hires", name: item.fullResWavName, path: item.fullResWavPath, fileType: "audio" });
      }
      if (item.originalPath && item.originalName) {
        assoc.push({ id: "audio-orig", name: item.originalName, path: item.originalPath, fileType: "audio" });
      }

      const profile: TrackProfile = {
        dbVolume: 0,
        speed: 1.0,
        pitch: 0,
        pitchCents: 0,
        eqBass: 0,
        eqMid: 0,
        eqTreble: 0,
        isEqBypassed: false,
        isCompressorBypassed: false,
        compressorThreshold: 0,
        compressorRatio: 1.0,
        compressorMakeup: 0,
        markers: [],
        nextMarkerId: 1,
        regions: [],
        nextRegionId: 1,
        associatedVersions: [],
        pdfChartPath: item.pdfPath || "",
        pdfChartName: item.pdfName || "",
        associatedFiles: assoc,
        notes: item.notesMarkdown || "",
        lyrics: "",
        notesViewMode: "preview",
        lyricsViewMode: "edit",
        eqNodes: [
          { id: "node-1", name: "Low Shelf", filterType: "LowShelf", freq: 100, gainDb: 0, q: 0.707, enabled: true, color: "#3b99fc" },
          { id: "node-2", name: "Mid Bell", filterType: "Peaking", freq: 1000, gainDb: 0, q: 1.0, enabled: true, color: "#30d158" },
          { id: "node-3", name: "High Shelf", filterType: "HighShelf", freq: 8000, gainDb: 0, q: 0.707, enabled: true, color: "#ff9500" }
        ],
        compStage1: { ...compStage1 },
        compStage2: { ...compStage2 },
        compRouting: "Series",
        compParallelBlend: 0.5
      };
      store[targetPath] = profile;
    });

    try {
      localStorage.setItem("th_track_profiles", JSON.stringify(store));
    } catch (e) {}

    // Load first track if available
    if (newItems.length > 0 && newItems[0].path && !newItems[0].isPlaceholder) {
      loadAudioPath(newItems[0].path, "main", true);
    }

    checkPlaylistHealthStatus();
    showSetlistModal = false;
  }

  async function checkPlaylistHealthStatus() {
    if (playlistItems.length === 0) {
      missingTracksCount = 0;
      return;
    }
    try {
      const realPaths = playlistItems.map(p => p.path || "");
      const existsList: boolean[] = await invoke("check_files_exist", { paths: realPaths });
      let missingCount = 0;
      existsList.forEach((ex, idx) => {
        const item = playlistItems[idx];
        if (!ex || item.isPlaceholder || !item.path || item.path.startsWith("[unlinked]")) {
          item.missingAudio = true;
          missingCount++;
        } else {
          item.missingAudio = false;
        }
      });
      playlistItems = [...playlistItems];
      missingTracksCount = missingCount;
    } catch (e) {
      console.error("Health check error:", e);
    }
  }

  async function openRepairModal() {
    showRepairModal = true;
    isScanningHealth = true;
    try {
      const realPaths = playlistItems.map(p => p.path || "");
      const existsList: boolean[] = await invoke("check_files_exist", { paths: realPaths });
      const libraries = await scanLibraryAssets();
      const allAudio = [...libraries.aacFiles, ...libraries.hiresFiles, ...libraries.origFiles];
      playlistHealth = analyzePlaylistHealth(playlistItems, existsList, allAudio);
    } catch (e) {
      console.error("Repair check error:", e);
    } finally {
      isScanningHealth = false;
    }
  }

  function applyRepairs() {
    const store = getProfilesStore();
    playlistHealth.forEach(h => {
      if (h.suggestedReplacement && (!h.exists || h.isPlaceholder)) {
        const item = playlistItems[h.index];
        if (item) {
          const oldPath = item.path;
          const newPath = h.suggestedReplacement.path;
          item.path = newPath;
          item.name = h.suggestedReplacement.name;
          item.isPlaceholder = false;
          item.missingAudio = false;

          // Migrate cached profile if present according to selective options
          const oldProfile = store[oldPath];
          if (oldProfile) {
            const newProfile: TrackProfile = {
              ...oldProfile,
              markers: repairMigrateMarkers ? (oldProfile.markers ? oldProfile.markers.map(m => ({ ...m })) : []) : [],
              nextMarkerId: repairMigrateMarkers ? (oldProfile.nextMarkerId || 1) : 1,
              regions: repairMigrateRegions ? (oldProfile.regions ? oldProfile.regions.map(r => ({ ...r })) : []) : [],
              nextRegionId: repairMigrateRegions ? (oldProfile.nextRegionId || 1) : 1,
              notes: repairMigrateNotes ? (oldProfile.notes || "") : "",
              lyrics: repairMigrateNotes ? (oldProfile.lyrics || "") : "",
              dbVolume: repairMigrateVolume ? (oldProfile.dbVolume ?? 0.0) : 0.0,
              volumeEnvelopeNodes: repairMigrateEnvelope ? (oldProfile.volumeEnvelopeNodes ? oldProfile.volumeEnvelopeNodes.map(n => ({ ...n })) : []) : [],
              isEnvelopeEnabled: repairMigrateEnvelope ? (oldProfile.isEnvelopeEnabled ?? true) : true,
              isEqBypassed: repairMigrateEq ? (oldProfile.isEqBypassed ?? false) : false,
              eqNodes: repairMigrateEq && oldProfile.eqNodes ? oldProfile.eqNodes.map(n => ({ ...n })) : undefined,
              eqBass: repairMigrateEq ? (oldProfile.eqBass ?? 0) : 0,
              eqMid: repairMigrateEq ? (oldProfile.eqMid ?? 0) : 0,
              eqTreble: repairMigrateEq ? (oldProfile.eqTreble ?? 0) : 0,
              isCompressorBypassed: repairMigrateComp ? (oldProfile.isCompressorBypassed ?? true) : true,
              compStage1: repairMigrateComp && oldProfile.compStage1 ? { ...oldProfile.compStage1 } : undefined,
              compStage2: repairMigrateComp && oldProfile.compStage2 ? { ...oldProfile.compStage2 } : undefined
            };
            store[newPath] = newProfile;
          }
        }
      }
    });

    playlistItems = [...playlistItems];
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
    flushProfilesToLocalStorage();

    checkPlaylistHealthStatus();
    showRepairModal = false;
  }

  // Crossfade Editor Modal State
  let showXfadeModal: boolean = false;
  let editingXfadeRegion: Region | null = null;
  let editingXfadeMs: number = 5;

  function openXfadeModal(region: Region) {
    editingXfadeRegion = region;
    editingXfadeMs = region.crossfadeMs ?? 5;
    showXfadeModal = true;
  }

  function applyXfadeModal() {
    if (editingXfadeRegion) {
      editingXfadeRegion.crossfadeMs = Math.max(0, Math.min(100, Math.round(editingXfadeMs)));
      regions = [...regions];
      syncRegionsToEngine();
      saveCurrentTrackProfile(filePath);
    }
    showXfadeModal = false;
    editingXfadeRegion = null;
  }

  function closeXfadeModal() {
    showXfadeModal = false;
    editingXfadeRegion = null;
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
        isInverted: shouldInvertPdfByDefault()
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
    volumeEnvelopeNodes?: VolumeEnvelopeNode[];
    isEnvelopeEnabled?: boolean;
    primarySongTrackPath?: string | null;
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
    fileTags?: Record<string, string[]>;
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

  /**
   * Bi-directionally synchronizes a song collection across all member audio files.
   * If File A associates File B and Doc C, then File B also has File A and Doc C as associated files,
   * with shared rehearsal markers, notes, lyrics, and sheet music.
   */
  function syncBidirectionalCollection(activeTrackPath: string) {
    if (!activeTrackPath) return;
    const store = getProfilesStore();
    const currentProfile = store[activeTrackPath];
    if (!currentProfile) return;

    const primaryPath = primarySongTrackPath || currentProfile.primarySongTrackPath || activeTrackPath;

    // 1. Gather all unique files belonging to this song collection
    const allFilesMap = new Map<string, AssociatedFileItem>();

    // Non-audio files from active associatedFiles (e.g. PDFs, charts)
    for (const f of associatedFiles) {
      if (f.fileType !== 'audio' && f.path) {
        allFilesMap.set(f.path, { ...f });
      }
    }

    // Audio files currently in associatedFiles
    for (const f of associatedFiles) {
      if (f.fileType === 'audio' && f.path) {
        allFilesMap.set(f.path, { ...f });
      }
    }

    // Active track itself
    if (!allFilesMap.has(activeTrackPath)) {
      const isPrimary = activeTrackPath === primaryPath;
      allFilesMap.set(activeTrackPath, {
        id: "track-active-" + activeTrackPath.replace(/[^a-zA-Z0-9]/g, "_"),
        name: activeTrackPath.split("/").pop() || activeTrackPath,
        path: activeTrackPath,
        fileType: "audio",
        role: isPrimary ? "main" : (isLosslessAudio(activeTrackPath) ? "hires" : "orig")
      });
    }

    // Primary default track (if different from activeTrackPath)
    if (primaryPath && primaryPath !== activeTrackPath && !allFilesMap.has(primaryPath)) {
      allFilesMap.set(primaryPath, {
        id: "track-primary-" + primaryPath.replace(/[^a-zA-Z0-9]/g, "_"),
        name: primaryPath.split("/").pop() || primaryPath,
        path: primaryPath,
        fileType: "audio",
        role: isLosslessAudio(primaryPath) ? "hires" : "main"
      });
    }

    const allAudioItems = Array.from(allFilesMap.values()).filter(f => f.fileType === 'audio');
    const allNonAudioItems = Array.from(allFilesMap.values()).filter(f => f.fileType !== 'audio');

    // If only 1 audio file and 0 non-audio files, nothing to synchronize to peers
    if (allAudioItems.length <= 1 && allNonAudioItems.length === 0) {
      currentProfile.primarySongTrackPath = primaryPath;
      flushProfilesToLocalStorage();
      return;
    }

    // Propagate to every audio file in the song collection
    for (const audioMember of allAudioItems) {
      if (audioMember.path === activeTrackPath) {
        currentProfile.primarySongTrackPath = primaryPath;
        continue;
      }

      // For peer audio member: its associatedFiles should be all other items in the collection
      const peerAssociatedFiles: AssociatedFileItem[] = [
        ...allNonAudioItems.map(f => ({ ...f })),
        ...allAudioItems.filter(f => f.path !== audioMember.path).map(f => ({ ...f }))
      ];

      const peerProfile = store[audioMember.path] || ({} as TrackProfile);
      store[audioMember.path] = {
        ...peerProfile,
        primarySongTrackPath: primaryPath,
        associatedFiles: peerAssociatedFiles,
        pdfChartPath: pdfChartPath || peerProfile.pdfChartPath || "",
        pdfChartName: pdfChartName || peerProfile.pdfChartName || "",
        markers: markers.map(m => ({ ...m })),
        nextMarkerId,
        regions: regions.map(r => ({ ...r })),
        nextRegionId,
        notes: songNotes || peerProfile.notes || "",
        lyrics: songLyrics || peerProfile.lyrics || "",
        notesViewMode,
        lyricsViewMode,
        lastCenterTab: activeCenterTab,
        fileTags: { ...(peerProfile.fileTags || {}), ...(currentProfile.fileTags || {}), ...activeSongTags }
      };
    }

    flushProfilesToLocalStorage();
  }

  let trackProfileDebounceTimer: any = null;

  function saveCurrentTrackProfile(trackPath: string | null, immediate = false) {
    if (!trackPath) return;
    if (!immediate) {
      if (trackProfileDebounceTimer) clearTimeout(trackProfileDebounceTimer);
      trackProfileDebounceTimer = setTimeout(() => {
        saveCurrentTrackProfile(trackPath, true);
      }, 350);
      return;
    }

    const store = getProfilesStore();
    const existing = store[trackPath] || ({} as TrackProfile);
    const primary = primarySongTrackPath || existing.primarySongTrackPath || trackPath;

    store[trackPath] = {
      ...existing,
      primarySongTrackPath: primary,
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
      volumeEnvelopeNodes: volumeEnvelopeNodes.map(n => ({ ...n })),
      isEnvelopeEnabled,
      associatedFiles: associatedFiles.map(a => ({ ...a })),
      pdfChartPath,
      pdfChartName,
      associatedVersions: associatedVersions.map(v => ({ ...v })),
      alternateTrackPath: alternateTrack ? alternateTrack.path : null,
      notes: songNotes,
      lyrics: songLyrics,
      notesViewMode,
      lyricsViewMode,
      lastCenterTab: activeCenterTab,
      fileTags: { ...(existing.fileTags || {}), ...activeSongTags }
    };

    // Propagate bidirectional associations across all audio peers in this collection
    syncBidirectionalCollection(trackPath);
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
    const unlinkedItem = associatedFiles.find(f => f.id === id);
    associatedFiles = associatedFiles.filter(f => f.id !== id);
    if (unlinkedItem && unlinkedItem.path) {
      const store = getProfilesStore();
      if (store[unlinkedItem.path]) {
        store[unlinkedItem.path].associatedFiles = [];
        store[unlinkedItem.path].primarySongTrackPath = null;
      }
    }
    saveCurrentTrackProfile(filePath, true);
  }

  function handleUvrSeparationComplete(files: Array<{ name: string; path: string; role: string; fileType: string }>) {
    let newlyAdded = 0;
    for (const file of files) {
      if (!associatedFiles.some(f => f.path === file.path)) {
        associatedFiles.push({
          id: "uvr_" + Date.now() + "_" + Math.random().toString(36).substring(2, 6),
          name: file.name,
          path: file.path,
          fileType: "audio",
          role: file.role,
        });
        newlyAdded++;
      }
    }
    if (newlyAdded > 0) {
      associatedFiles = [...associatedFiles];
      saveCurrentTrackProfile(filePath, true);
      uvrSuccessMessage = `Separation complete! Added ${newlyAdded} stem${newlyAdded > 1 ? 's' : ''} to Associated Files.`;
      setTimeout(() => {
        uvrSuccessMessage = null;
      }, 7000);
    }
  }

  // Audio file tags system (auto-detected and user-toggled via right-click)
  let activeSongTags: Record<string, string[]> = {};

  const AUDIO_TAG_OPTIONS = [
    { id: "original", label: "Original", color: "#d084ff" },
    { id: "vocals_only", label: "Vocals only", color: "#30d158" },
    { id: "lead_vocal", label: "Lead Vocal", color: "#ffd60a" },
    { id: "background_vocals", label: "Background Vocals", color: "#64d2ff" },
    { id: "iso_track", label: "Iso Track", color: "#38bdf8" },
    { id: "track", label: "Track", color: "#ff9f0a" }
  ];

  function getAutoDetectedTags(path: string, name?: string, roleHint?: string): string[] {
    const text = ((path || "") + " " + (name || "")).toLowerCase();
    const fileNameOnly = (name || (path ? path.split("/").pop()?.split("\\").pop() : "") || "").toLowerCase();
    const detected: string[] = [];

    // 1. Original Artist / Reference Recording
    if (
      roleHint === "orig" ||
      text.includes("audio-orig") ||
      text.includes("original") ||
      text.includes("[original]") ||
      text.includes("(original)") ||
      text.includes("album version") ||
      text.includes("artist version") ||
      text.includes("album cut") ||
      text.includes("reference recording") ||
      text.includes("original mix")
    ) {
      detected.push("Original");
    }

    // 2. Lead Vocal Track
    if (
      roleHint === "lead" ||
      text.includes("lead vocal") ||
      text.includes("lead-vocal") ||
      text.includes("lead_vocal") ||
      text.includes("(lead vocals)") ||
      text.includes("[lead vocals]") ||
      text.includes("(lead vocal)") ||
      text.includes("[lead vocal]") ||
      text.includes("lead vox") ||
      text.includes("lead_vox") ||
      text.includes("(lead)") ||
      text.includes("[lead]") ||
      text.includes("_lead.") ||
      text.includes("- lead.") ||
      text.includes(" lead.") ||
      text.includes("lead voice")
    ) {
      detected.push("Lead Vocal");
    }

    // 3. Background Vocals / Backing Vocals Track
    // Distinguish "Backing Track" (instrumental) from "Backing Vocals" / "BGV"
    const isBackingTrackText = text.includes("backing track") || 
      text.includes("backing-track") || 
      text.includes("backing_track") ||
      text.includes("performance track");

    const hasBgvKeyword = 
      roleHint === "backings" ||
      text.includes("backing vocal") ||
      text.includes("backing-vocal") ||
      text.includes("backing_vocal") ||
      text.includes("background vocal") ||
      text.includes("background-vocal") ||
      text.includes("background_vocal") ||
      text.includes("backup vocal") ||
      text.includes("backup-vocal") ||
      text.includes("backup_vocal") ||
      text.includes("bgv") ||
      text.includes("bgvs") ||
      text.includes("bg vox") ||
      text.includes("backing vox") ||
      text.includes("harmonies") ||
      text.includes("harmony vocals") ||
      text.includes("(backings)") ||
      text.includes("[backings]") ||
      (!isBackingTrackText && (
        text.includes("backings") ||
        text.includes("(backup)") ||
        text.includes("[backup]") ||
        text.includes("backing.") ||
        text.includes("- backing") ||
        text.includes("_backing")
      ));

    if (hasBgvKeyword) {
      detected.push("Background Vocals");
    }

    // 4. Vocals only (Master Isolated Vocal Stem / Acapella)
    const hasVocalsOnlyKeyword = 
      roleHint === "vocals" ||
      text.includes("vocals only") || 
      text.includes("(vocals)") || 
      text.includes("[vocals]") || 
      text.includes("vocals ensemble") || 
      text.includes("isolated_vocals") || 
      text.includes("isolated vocals") || 
      text.includes("acapella") || 
      text.includes("acappella") || 
      fileNameOnly.includes("vocals.") ||
      fileNameOnly.includes("_vocals.") ||
      fileNameOnly.includes(" vocals.") ||
      fileNameOnly.includes("-vocals.");

    if (hasVocalsOnlyKeyword) {
      if (!detected.includes("Lead Vocal") && !detected.includes("Background Vocals")) {
        detected.push("Vocals only");
      }
    }

    // 5. Iso Track (Isolated Instrumental Stem from Vocal Separation)
    const hasIsoTrackKeyword = 
      roleHint === "iso_track" ||
      text.includes("iso track") ||
      text.includes("iso-track") ||
      text.includes("iso_track") ||
      text.includes("(iso track)") ||
      text.includes("[iso track]") ||
      text.includes("(instrumental)") ||
      text.includes("[instrumental]") ||
      fileNameOnly.includes("iso track") ||
      fileNameOnly.includes("instrumental.");

    if (hasIsoTrackKeyword) {
      detected.push("Iso Track");
    }

    // 6. Track (Accompaniment / Backing Track / Instrumental / Karaoke)
    const hasTrackKeyword = 
      roleHint === "main" ||
      text.includes("backing track") ||
      text.includes("performance track") ||
      text.includes("split track") ||
      text.includes("accompaniment") ||
      text.includes("instrumental") ||
      text.includes("karaoke") ||
      text.includes("minus one") ||
      text.includes("minus 1") ||
      text.includes("no vox") ||
      text.includes("no vocals") ||
      text.includes("(track)") ||
      text.includes("[track]") ||
      text.includes("_track.") ||
      text.includes("-track.") ||
      text.includes(" track.") ||
      text.includes("- track.") ||
      text.includes("- track ") ||
      text.includes("rehearsal track") ||
      text.includes("stage track");

    if (hasTrackKeyword && !detected.includes("Iso Track")) {
      detected.push("Track");
    } else if (
      // If no tag detected yet, check if this is the designated primary track
      (roleHint === "main" || (primarySongTrackPath && path === primarySongTrackPath)) &&
      !detected.includes("Original") &&
      !detected.includes("Vocals only") &&
      !detected.includes("Lead Vocal") &&
      !detected.includes("Background Vocals") &&
      !detected.includes("Iso Track")
    ) {
      detected.push("Track");
    }

    return detected;
  }

  function getFileTags(trackPath: string, trackName?: string, roleHint?: string): string[] {
    if (!trackPath) return [];
    // 1. In-memory active song tags
    if (activeSongTags && Array.isArray(activeSongTags[trackPath])) {
      return activeSongTags[trackPath];
    }
    const store = getProfilesStore();
    // 2. Exact match in profile store
    if (store[trackPath]?.fileTags && Array.isArray(store[trackPath].fileTags[trackPath])) {
      activeSongTags[trackPath] = [...store[trackPath].fileTags[trackPath]];
      return activeSongTags[trackPath];
    }
    // 3. Look under active song's profile key
    if (filePath && store[filePath]?.fileTags && Array.isArray(store[filePath].fileTags[trackPath])) {
      activeSongTags[trackPath] = [...store[filePath].fileTags[trackPath]];
      return activeSongTags[trackPath];
    }
    // 4. Look under primarySongTrackPath profile key
    if (primarySongTrackPath && store[primarySongTrackPath]?.fileTags && Array.isArray(store[primarySongTrackPath].fileTags[trackPath])) {
      activeSongTags[trackPath] = [...store[primarySongTrackPath].fileTags[trackPath]];
      return activeSongTags[trackPath];
    }

    // 5. Look up associated file role / ID if not explicitly passed
    let effectiveRole = roleHint;
    if (!effectiveRole && associatedFiles) {
      const match = associatedFiles.find(f => f.path === trackPath);
      if (match) {
        effectiveRole = match.role || (match.id === 'audio-orig' ? 'orig' : undefined);
      }
    }
    if (!effectiveRole && primarySongTrackPath && trackPath === primarySongTrackPath) {
      effectiveRole = "main";
    }

    return getAutoDetectedTags(trackPath, trackName, effectiveRole);
  }

  function toggleFileTag(trackPath: string, tag: string) {
    if (!trackPath || !tag) return;
    const currentTags = [...getFileTags(trackPath)];
    const index = currentTags.indexOf(tag);
    if (index >= 0) {
      currentTags.splice(index, 1);
    } else {
      currentTags.push(tag);
    }
    activeSongTags = {
      ...activeSongTags,
      [trackPath]: currentTags
    };

    const store = getProfilesStore();
    const targetPath = filePath || primarySongTrackPath || trackPath;
    if (store[targetPath]) {
      store[targetPath].fileTags = {
        ...(store[targetPath].fileTags || {}),
        ...activeSongTags
      };
    }
    if (filePath) {
      saveCurrentTrackProfile(filePath, true);
    } else {
      flushProfilesToLocalStorage();
    }
  }

  // Waveform Dynamic Height Resizing State & Handlers
  const savedWaveformHeight = parseInt(localStorage.getItem("th_waveform_height") || "256", 10);
  let waveformHeight: number = !isNaN(savedWaveformHeight) && savedWaveformHeight >= 100 && savedWaveformHeight <= 650 ? savedWaveformHeight : 256;
  let isResizingWaveform = false;
  let resizeWaveformStartY = 0;
  let resizeWaveformStartHeight = 256;

  function updateWaveformHeight(newH: number) {
    const clamped = Math.round(Math.max(100, Math.min(650, newH)));
    if (clamped === waveformHeight) return;
    waveformHeight = clamped;
    invalidateWaveformCaches();
    tick().then(() => {
      drawMainWaveform();
      drawOverviewWaveform();
    });
  }

  function startWaveformResize(e: MouseEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    isResizingWaveform = true;
    resizeWaveformStartY = e.clientY;
    resizeWaveformStartHeight = waveformHeight;

    const onMouseMove = (moveEvent: MouseEvent) => {
      const deltaY = resizeWaveformStartY - moveEvent.clientY;
      updateWaveformHeight(resizeWaveformStartHeight + deltaY);
    };

    const onMouseUp = () => {
      isResizingWaveform = false;
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      localStorage.setItem("th_waveform_height", waveformHeight.toString());
      invalidateWaveformCaches();
      drawMainWaveform();
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }

  function resetWaveformHeight() {
    updateWaveformHeight(256);
    localStorage.setItem("th_waveform_height", "256");
  }

  function getTrackWatermarkLabel(): string {
    if (!filePath) return "";
    const tags = getFileTags(filePath);
    if (tags.length > 0) {
      return tags.join(" • ").toUpperCase();
    }
    const activeAssoc = associatedFiles.find(f => f.path === filePath);
    if (activeAssoc) {
      if (activeAssoc.role === 'lead' || activeAssoc.name.toLowerCase().includes('lead vocal')) return "LEAD VOCAL";
      if (activeAssoc.role === 'backings' || activeAssoc.name.toLowerCase().includes('backing vocal')) return "BACKGROUND VOCALS";
      if (activeAssoc.role === 'vocals' || activeAssoc.name.toLowerCase().includes('vocals ensemble') || activeAssoc.name.toLowerCase().includes('(vocals)')) return "VOCALS ONLY";
      if (isLosslessAudio(activeAssoc.path || activeAssoc.name)) return "HIGH-RES MASTER";
      if (activeAssoc.id === 'audio-orig' || activeAssoc.name.toLowerCase().includes('original')) return "ORIGINAL";
      return activeAssoc.name.toUpperCase();
    }
    if (isLosslessAudio(filePath)) return "HIGH-RES MASTER";
    return "DEFAULT MIX";
  }

  async function loadAudioVersion(path: string, preservePlayhead = true) {
    if (!path) return;
    if (filePath === path) return;

    const wasPlaying = isPlaying;
    const sourceTime = currentTime;

    // Track A/B history
    if (filePath) {
      abPreviousTrackPath = filePath;
    }

    if (!primarySongTrackPath) {
      primarySongTrackPath = filePath || path;
    }

    await loadAudioPath(path, "main", true);

    // If preservePlayhead and we had playback time, seek to keep exact same time position
    if (preservePlayhead && sourceTime > 0 && sourceTime < duration) {
      currentTime = sourceTime;
      progress = duration > 0 ? currentTime / duration : 0;
      await invoke("seek", { seconds: sourceTime });
    }

    if (wasPlaying) {
      await invoke("play");
      isPlaying = true;
    }

    uvrSuccessMessage = `Loaded "${fileName}" into waveform`;
    setTimeout(() => {
      if (uvrSuccessMessage && uvrSuccessMessage.includes("Loaded")) uvrSuccessMessage = null;
    }, 3500);
  }

  async function loadAssociatedTrackIntoActive(item: AssociatedFileItem) {
    if (!item || item.fileType !== 'audio' || !item.path) return;
    await loadAudioVersion(item.path, true);
  }

  async function triggerAbCompare() {
    // Determine the A/B flip target
    let targetPath: string | null = null;

    if (primarySongTrackPath && filePath === primarySongTrackPath) {
      // Currently on primary mix, flip to the previously used stem
      targetPath = abPreviousTrackPath;
    } else if (primarySongTrackPath && filePath !== primarySongTrackPath) {
      // Currently on a stem, flip back to primary mix
      targetPath = primarySongTrackPath;
    } else if (abPreviousTrackPath && filePath !== abPreviousTrackPath) {
      targetPath = abPreviousTrackPath;
    }

    // Fallback: if no flip target recorded yet, pick the first other associated audio file
    if (!targetPath || targetPath === filePath) {
      const candidate = associatedFiles.find(f => f.fileType === 'audio' && f.path !== filePath);
      if (candidate) {
        targetPath = candidate.path;
      }
    }

    if (targetPath && targetPath !== filePath) {
      await loadAudioVersion(targetPath, true);
    }
  }

  function setDefaultSongTrack(item: AssociatedFileItem) {
    if (!item || item.fileType !== 'audio' || !item.path) return;
    primarySongTrackPath = item.path;

    // Update playlist item path if this song is in the playlist
    if (selectedPlaylistIndex >= 0 && selectedPlaylistIndex < playlistItems.length) {
      playlistItems[selectedPlaylistIndex].path = item.path;
      playlistItems = [...playlistItems];
      localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
    }

    saveCurrentTrackProfile(filePath, true);

    uvrSuccessMessage = `Set "${item.name}" as Default Track for this song`;
    setTimeout(() => {
      if (uvrSuccessMessage && uvrSuccessMessage.includes("Set")) uvrSuccessMessage = null;
    }, 4000);
  }

  function handleAssocFileContextMenu(e: MouseEvent, item: AssociatedFileItem) {
    e.preventDefault();
    e.stopPropagation();
    contextMenuType = "assoc-file";
    contextMenuTargetAssoc = item;
    contextMenuX = e.clientX;
    contextMenuY = e.clientY;
    showContextMenu = true;
  }

  async function checkAndApplyHighResPreference(targetProfileAssocFiles?: AssociatedFileItem[]) {
    if (!preferHighResAudio) return false;
    const list = targetProfileAssocFiles || associatedFiles;
    if (!list || list.length === 0) return false;

    // Look for a lossless audio file
    const hiresItem = list.find(f => f.fileType === 'audio' && (f.role === 'hires' || isLosslessAudio(f.path || f.name)));
    if (!hiresItem || !hiresItem.path) return false;

    try {
      const isDownloaded: boolean = await invoke("is_file_downloaded", { path: hiresItem.path });
      if (isDownloaded) {
        if (filePath !== hiresItem.path) {
          console.log("Preferring downloaded high-res track:", hiresItem.path);
          await loadAudioPath(hiresItem.path, "main", true);
          return true;
        }
      } else {
        console.log("High-res file exists but is not downloaded locally:", hiresItem.path);
      }
    } catch (e) {
      console.error("Failed to check high-res download status:", e);
    }
    return false;
  }

  async function togglePreferHighResAudio() {
    preferHighResAudio = !preferHighResAudio;
    localStorage.setItem("th_prefer_high_res", preferHighResAudio ? "true" : "false");
    
    if (preferHighResAudio) {
      const applied = await checkAndApplyHighResPreference();
      if (applied) {
        uvrSuccessMessage = "Switched to downloaded High-Res Lossless audio!";
        setTimeout(() => { if (uvrSuccessMessage?.includes("High-Res")) uvrSuccessMessage = null; }, 4000);
      } else {
        const hasLossless = associatedFiles.some(f => f.fileType === 'audio' && (f.role === 'hires' || isLosslessAudio(f.path || f.name)));
        if (hasLossless) {
          alert("High-Res audio found, but it is currently online-only in Dropbox and not downloaded to this Mac.");
        } else {
          uvrSuccessMessage = "Prefer High-Res enabled (will load high-res when available & downloaded).";
          setTimeout(() => { if (uvrSuccessMessage?.includes("Prefer High-Res")) uvrSuccessMessage = null; }, 4000);
        }
      }
    } else {
      // If turned off, check if currently playing high-res and we have an AAC or other track to revert to
      if (filePath && isLosslessAudio(filePath)) {
        const standardVersion = associatedFiles.find(f => f.fileType === 'audio' && !isLosslessAudio(f.path || f.name) && (f.role === 'main' || !f.role || f.role === 'orig'));
        if (standardVersion && standardVersion.path) {
          await loadAudioPath(standardVersion.path, "main", true);
        }
      }
      uvrSuccessMessage = "Prefer High-Res disabled. Using standard AAC main tracks.";
      setTimeout(() => { if (uvrSuccessMessage?.includes("Prefer High-Res")) uvrSuccessMessage = null; }, 4000);
    }
  }

  async function triggerUvrSeparation(mode: 'ensemble_vocals' | 'lead_backups', trackPath?: string) {
    const target = trackPath || filePath;
    if (!target) {
      alert("Please select or load an audio track first.");
      return;
    }
    uvrSeparating = true;
    uvrPercent = 0;
    uvrStage = mode === "ensemble_vocals" ? "Initializing 4-Model Ensemble..." : "Initializing 5HP Karaoke...";
    uvrCurrentTarget = target.split("/").pop() || target;
    uvrErrorMessage = null;
    uvrSuccessMessage = null;

    try {
      const res: any = await invoke("run_uvr_separation", {
        trackPath: target,
        mode,
        outputDir: null
      });
      if (res && res.files && res.files.length > 0) {
        handleUvrSeparationComplete(res.files);
      }
    } catch (e: any) {
      console.error("UVR separation failed:", e);
      uvrErrorMessage = typeof e === "string" ? e : (e.message || "UVR separation error");
    } finally {
      uvrSeparating = false;
    }
  }

  function requestMoveToTrash(item: AssociatedFileItem) {
    itemToTrash = item;
    trashErrorMessage = null;
    showTrashModal = true;
  }

  async function executeMoveToTrash() {
    if (!itemToTrash) return;
    trashLoading = true;
    trashErrorMessage = null;
    try {
      await invoke("move_file_to_trash", { path: itemToTrash.path });
      const trashedPath = itemToTrash.path;
      // Remove from associatedFiles
      associatedFiles = associatedFiles.filter(f => f.id !== itemToTrash!.id);
      if (trashedPath) {
        const store = getProfilesStore();
        if (store[trashedPath]) {
          delete store[trashedPath];
        }
      }
      if (trashedPath === filePath) {
        const nextAudio = associatedFiles.find(f => f.fileType === 'audio');
        if (nextAudio && nextAudio.path) {
          await loadAudioPath(nextAudio.path, "main", true);
        } else if (primarySongTrackPath && primarySongTrackPath !== trashedPath) {
          await loadAudioPath(primarySongTrackPath, "main", true);
        } else {
          handleStop();
          filePath = "";
          fileName = "";
        }
      }
      saveCurrentTrackProfile(filePath, true);
      showTrashModal = false;
      itemToTrash = null;
    } catch (e: any) {
      console.error("Move to trash failed:", e);
      trashErrorMessage = typeof e === "string" ? e : (e.message || "Failed to move file to trash");
    } finally {
      trashLoading = false;
    }
  }


  function getActivePdfContainer(): HTMLElement | null {
    if (activeCenterTab.startsWith("pdf-")) {
      return document.getElementById("pdf-container-" + activePdfTabId);
    } else if (activeCenterTab === "pdf") {
      return pdfContainer;
    }
    return document.querySelector(".pdf-scroll-column") as HTMLElement | null;
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
        container.appendChild(pageWrapper);

        await page.render({ canvasContext: ctx, viewport }).promise;
      }

      tab.isLoading = false;
      openPdfTabs = [...openPdfTabs];
      tick().then(() => renderPdfMarkerBadges());
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

    const container = document.getElementById("pdf-container-" + tab.id);
    if (!container) return;
    const cards = container.querySelectorAll(".pdf-page-card");

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
        handlePdfPageDrop(e, pageNum, card);
        break;
      }
    }
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
    const cards = document.querySelectorAll(".pdf-page-card");
    if (!cards || cards.length === 0) return;
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
    let profile = store[trackPath];
    let parentKey: string | null = null;

    // Check if this track belongs to an existing collection in the store
    // (If profile is missing OR its associatedFiles is empty, check if another profile has trackPath in its associatedFiles)
    if (!profile || !profile.associatedFiles || profile.associatedFiles.length === 0) {
      for (const [key, p] of Object.entries(store)) {
        if (key !== trackPath && p.associatedFiles && p.associatedFiles.some(f => f.path === trackPath)) {
          profile = p;
          parentKey = key;
          break;
        }
      }
    }

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
      volumeEnvelopeNodes = Array.isArray(profile.volumeEnvelopeNodes) ? [...profile.volumeEnvelopeNodes] : [];
      isEnvelopeEnabled = typeof profile.isEnvelopeEnabled === "boolean" ? profile.isEnvelopeEnabled : true;
      selectedEnvelopeNodeId = null;

      pdfChartPath = profile.pdfChartPath || "";
      pdfChartName = profile.pdfChartName || (pdfChartPath ? (pdfChartPath.split("/").pop() || "") : "");
      associatedVersions = Array.isArray(profile.associatedVersions) ? profile.associatedVersions : [];
      
      // Populate bidirectional associatedFiles and primary track
      if (parentKey) {
        primarySongTrackPath = profile.primarySongTrackPath || parentKey;
        const parentItem: AssociatedFileItem = {
          id: "track-parent-" + parentKey.replace(/[^a-zA-Z0-9]/g, "_"),
          name: parentKey.split("/").pop() || parentKey,
          path: parentKey,
          fileType: "audio",
          role: parentKey === primarySongTrackPath ? "main" : (isLosslessAudio(parentKey) ? "hires" : "orig")
        };
        const siblingItems = (profile.associatedFiles || []).filter(f => f.path !== trackPath);
        associatedFiles = [parentItem, ...siblingItems];
      } else {
        primarySongTrackPath = profile.primarySongTrackPath || trackPath;
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
        }
      }

      songNotes = profile.notes || "";
      songLyrics = profile.lyrics || "";
      notesViewMode = profile.notesViewMode || "edit";
      lyricsViewMode = profile.lyricsViewMode || "edit";
      activeSongTags = profile.fileTags ? { ...profile.fileTags } : {};

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

      // Sync and persist this track's profile in the collection
      saveCurrentTrackProfile(trackPath, true);

      // If Prefer High-Res is enabled, check if downloaded lossless version is available
      if (preferHighResAudio && !isLosslessAudio(trackPath)) {
        checkAndApplyHighResPreference(associatedFiles);
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
      volumeEnvelopeNodes = [];
      isEnvelopeEnabled = true;
      selectedEnvelopeNodeId = null;

      primarySongTrackPath = trackPath;
      pdfChartPath = "";
      pdfChartName = "";
      associatedVersions = [];
      associatedFiles = [];
      songNotes = "";
      songLyrics = "";
      notesViewMode = "edit";
      lyricsViewMode = "edit";
      activeSongTags = {};
      alternateTrack = null;
      activeCenterTab = "notes";
      saveCurrentTrackProfile(trackPath, true);
    }

    // Apply restored volume, speed, pitch, EQ, compressor, regions, and volume envelope directly to audio engine
    const linearVol = dbVolume <= -59.5 ? 0 : Math.pow(10, dbVolume / 20);
    await invoke("set_volume", { volume: linearVol });
    await invoke("set_speed", { speed });
    const totalSemitones = pitch + (pitchCents / 100.0);
    await invoke("set_pitch", { pitch: totalSemitones });
    await updateEqEngine();
    await updateCompressorEngine();
    await syncRegionsToEngine();
    await updateVolumeEnvelopeEngine();
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
    loadAppPreferences();

    // Restore Sidebar Widths
    const savedLeftWidth = localStorage.getItem("th_left_sidebar_width");
    if (savedLeftWidth) {
      const parsed = parseInt(savedLeftWidth, 10);
      if (!isNaN(parsed) && parsed >= 160 && parsed <= 600) leftSidebarWidth = parsed;
    }
    const savedRightWidth = localStorage.getItem("th_right_sidebar_width");
    if (savedRightWidth) {
      const parsed = parseInt(savedRightWidth, 10);
      if (!isNaN(parsed) && parsed >= 180 && parsed <= 700) rightSidebarWidth = parsed;
    }

    // Restore Setlist Import Modal Size
    const savedModalW = localStorage.getItem("th_setlist_modal_w");
    if (savedModalW) {
      const parsed = parseInt(savedModalW, 10);
      if (!isNaN(parsed) && parsed >= 550) setlistModalWidth = Math.min(parsed, window.innerWidth * 0.96);
    }
    const savedModalH = localStorage.getItem("th_setlist_modal_h");
    if (savedModalH) {
      const parsed = parseInt(savedModalH, 10);
      if (!isNaN(parsed) && parsed >= 400) setlistModalHeight = Math.min(parsed, window.innerHeight * 0.94);
    }

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
    if (savedInvert === "1") {
      isPdfInverted = true;
    } else if (savedInvert === "0") {
      isPdfInverted = false;
    } else {
      isPdfInverted = shouldInvertPdfByDefault();
    }

    const savedCenterTab = localStorage.getItem("th_last_center_tab") as "notes" | "lyrics" | "metadata" | "pdf" | null;
    if (savedCenterTab) activeCenterTab = savedCenterTab;

    const savedVersions = localStorage.getItem("th_associated_versions");
    if (savedVersions) associatedVersions = JSON.parse(savedVersions);

    // UVR Event Listeners
    listen<{ percent: number; stage: string }>("uvr-progress", (event) => {
      uvrSeparating = true;
      uvrPercent = event.payload.percent;
      uvrStage = event.payload.stage;
    });

    listen<{ success: boolean; files: Array<{ name: string; path: string; role: string; fileType: string }>; error?: string }>("uvr-complete", (event) => {
      uvrSeparating = false;
      uvrPercent = 100;
      uvrStage = "Done!";
      if (event.payload.files && event.payload.files.length > 0) {
        handleUvrSeparationComplete(event.payload.files);
      }
    });

    listen<string>("uvr-error", (event) => {
      uvrSeparating = false;
      uvrErrorMessage = event.payload;
    });


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
        if (isPlaying) {
          liveInputPeakL = typeof status.in_peak_l === "number" ? status.in_peak_l : -60.0;
          liveInputPeakR = typeof status.in_peak_r === "number" ? status.in_peak_r : -60.0;
          liveOutputPeakL = typeof status.out_peak_l === "number" ? status.out_peak_l : -60.0;
          liveOutputPeakR = typeof status.out_peak_r === "number" ? status.out_peak_r : -60.0;
          liveGainReductionDb = activeCompStageTab === 1 ? (status.gr_stage1 || 0.0) : (status.gr_stage2 || 0.0);
        } else {
          liveInputPeakL = -60.0;
          liveInputPeakR = -60.0;
          liveOutputPeakL = -60.0;
          liveOutputPeakR = -60.0;
          liveGainReductionDb = 0.0;
        }

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
          broadcastCurrentState();

          // Auto-scroll PDF to active anchored marker (12px below top)
          const activePdfCont = getActivePdfContainer();
          if (activePdfCont && isPlaying) {
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
                const card = activePdfCont.querySelector(`.pdf-page-card[data-page-num="${markerPage}"]`) as HTMLElement;
                if (card) {
                  const containerRect = activePdfCont.getBoundingClientRect();
                  const cardRect = card.getBoundingClientRect();
                  const cardTopRelativeToContainer = (cardRect.top - containerRect.top) + activePdfCont.scrollTop;
                  const markerYWithinCard = card.clientHeight * anchor.yPct;
                  const targetScrollTop = Math.max(0, cardTopRelativeToContainer + markerYWithinCard - 12);
                  activePdfCont.scrollTo({
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
    let resizeRafId: number | null = null;
    if (centerContentElement) {
      resizeObserver = new ResizeObserver(() => {
        if (resizeRafId !== null) return;
        resizeRafId = requestAnimationFrame(() => {
          resizeRafId = null;
          invalidateWaveformCaches();
          updateVisiblePeaks();
          drawMainWaveform();
          drawOverviewWaveform();
        });
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

    // Close context menu & color palette & cloud dropdown on window click
    const closeMenu = () => { 
      showContextMenu = false; 
      colorPaletteMarker = null;
      showCloudDropdown = false;
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
      else if (action === "open_preferences") openPreferencesModal();
      else if (action === "open_about") openAboutModal();
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

    // Global keyboard shortcuts: Space = Play/Pause/Load, Up/Down = Playlist/Browser Nav, Left/Right = Marker Jump, M = Add Marker, Enter = Stop & Return to 0, Cmd+Shift+E = Export Audio, Cmd+Shift+R = Remote Settings, Cmd+, = Preferences
    const handleKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }

      if ((e.metaKey || e.ctrlKey) && (e.key === "," || e.code === "Comma")) {
        e.preventDefault();
        openPreferencesModal();
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
        if (typeToJumpBuffer.length > 1) {
          performTypeToJump(typeToJumpBuffer);
        }
        clearTypeahead();
        // Standard Play/Pause toggle for currently loaded track
        handlePlayPause();
        return;
      } else if (e.code === "ArrowUp") {
        e.preventDefault();
        clearTypeahead();
        if (e.altKey && activeTab === "playlist" && selectedPlaylistIndex >= 0) {
          movePlaylistItem(selectedPlaylistIndex, selectedPlaylistIndex - 1);
          scrollSelectedPlaylistItemIntoView();
          return;
        }
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
        return;
      } else if (e.code === "ArrowDown") {
        e.preventDefault();
        clearTypeahead();
        if (e.altKey && activeTab === "playlist" && selectedPlaylistIndex >= 0) {
          movePlaylistItem(selectedPlaylistIndex, selectedPlaylistIndex + 1);
          scrollSelectedPlaylistItemIntoView();
          return;
        }
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
        return;
      } else if (e.code === "ArrowLeft") {
        e.preventDefault();
        clearTypeahead();
        jumpToPrevMarker();
        return;
      } else if (e.code === "ArrowRight") {
        e.preventDefault();
        clearTypeahead();
        jumpToNextMarker();
        return;
      } else if (e.code === "Enter" || e.code === "NumpadEnter") {
        e.preventDefault();
        if (typeToJumpBuffer.length > 1) {
          performTypeToJump(typeToJumpBuffer);
        }
        clearTypeahead();
        if (activeTab === "browser" && lastSelectedEntry) {
          if (lastSelectedEntry.is_dir) {
            loadBrowser(lastSelectedEntry.path);
          } else {
            loadAudioPath(lastSelectedEntry.path, "main", true).then(async () => {
              await invoke("play");
              isPlaying = true;
            });
          }
        } else if (activeTab === "playlist" && selectedPlaylistIndex >= 0 && selectedPlaylistIndex < playlistItems.length) {
          const highlighted = playlistItems[selectedPlaylistIndex];
          if (highlighted && highlighted.path !== filePath) {
            loadAudioPath(highlighted.path, "main", true).then(async () => {
              await invoke("play");
              isPlaying = true;
            });
          } else {
            handleStop();
          }
        } else {
          handleStop();
        }
        return;
      } else if (e.code === "Delete" || e.code === "Backspace") {
        clearTypeahead();
        if (selectedEnvelopeNodeId) {
          e.preventDefault();
          removeVolumeEnvelopeNode(selectedEnvelopeNodeId);
          return;
        }
        return;
      } else if (e.code === "Escape") {
        e.preventDefault();
        showContextMenu = false;
        colorPaletteMarker = null;
        cancelRenameMarker();
        selectedEnvelopeNodeId = null;
        clearTypeahead();
        handleStop();
        return;
      }

      // If active typeahead is buffering input, ANY printable character appends to the search buffer!
      if (typeToJumpBuffer.length > 0 && e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        handleTypeaheadKey(e.key);
        return;
      }

      // Hotkeys for audio waveform actions vs. typeahead in sidebar
      const isSidebarFocused = lastFocusedPane === "sidebar";

      if ((e.code === "KeyT" || e.key === "t" || e.key === "T") && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (isSidebarFocused) {
          handleTypeaheadKey(e.key);
        } else {
          triggerAbCompare();
        }
        return;
      }

      if ((e.code === "KeyM" || e.key === "m" || e.key === "M") && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (isSidebarFocused) {
          handleTypeaheadKey(e.key);
        } else {
          addMarker();
        }
        return;
      }

      if ((e.code === "KeyR" || e.key === "r" || e.key === "R") && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (isSidebarFocused) {
          handleTypeaheadKey(e.key);
        } else {
          createRegionFromSelectionOrMarkers();
        }
        return;
      }

      if ((e.code === "KeyX" || e.key === "x" || e.key === "X") && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (isSidebarFocused) {
          handleTypeaheadKey(e.key);
        } else {
          handleCutHotkey();
        }
        return;
      }

      if ((e.code === "KeyL" || e.key === "l" || e.key === "L") && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (isSidebarFocused) {
          handleTypeaheadKey(e.key);
        } else {
          handleLoopHotkey();
        }
        return;
      }

      // Standard printable characters (letters, numbers, symbols) -> initiate type-to-jump in sidebar!
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        lastFocusedPane = "sidebar";
        handleTypeaheadKey(e.key);
        return;
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("mousedown", handleContainerMousedown);

    return () => {
      clearInterval(statusInterval);
      if (resizeRafId !== null) cancelAnimationFrame(resizeRafId);
      if (resizeObserver) resizeObserver.disconnect();
      unlistenDragDrop.then(fn => fn());
      unlistenMenu.then(fn => fn());
      unlistenRemote.then(fn => fn());
      unlistenMidi.then(fn => fn());
      window.removeEventListener("click", closeMenu);
      window.removeEventListener("mousedown", handleContainerMousedown);
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
      if (contents.drives && contents.drives.length > 0) {
        systemDrives = contents.drives;
      }
      if (contents.cloud_folders && contents.cloud_folders.length > 0) {
        cloudFolders = contents.cloud_folders;
      }
      if (contents.root_name) {
        rootName = contents.root_name;
      }
      if (contents.home_path) {
        homePath = contents.home_path;
      }
      selectedFilePaths.clear();
      lastSelectedEntry = null;
      if (currentPath && currentPath !== rootName) {
        localStorage.setItem("th_last_dir", currentPath);
      }
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

  // Type-To-Jump & Typeahead Engine
  const TYPEAHEAD_REST_PERIOD_MS = 750;

  function cleanItemNameForSearch(name: string): string {
    if (!name) return "";
    const withoutExt = name.replace(/\.[a-zA-Z0-9]+$/, "");
    const withoutTrackNum = withoutExt.replace(/^([a-zA-Z]?\d+[\s\.\-_]+)+/, "");
    return withoutTrackNum.trim() || withoutExt.trim() || name.trim();
  }

  function matchesTypeahead(name: string, query: string): boolean {
    if (!name || !query) return false;
    const q = query.toLowerCase();
    const raw = name.toLowerCase();
    if (raw.startsWith(q)) return true;
    const cleaned = cleanItemNameForSearch(name).toLowerCase();
    if (cleaned.startsWith(q)) return true;
    if (raw.replace(/^the[\s\._-]+/, "").startsWith(q)) return true;
    if (cleaned.replace(/^the[\s\._-]+/, "").startsWith(q)) return true;
    return false;
  }

  function findMatchingBrowserIndex(query: string, startIndex = 0): number {
    if (!query || filteredEntries.length === 0) return -1;
    for (let i = startIndex; i < filteredEntries.length; i++) {
      if (matchesTypeahead(filteredEntries[i].name, query)) return i;
    }
    if (startIndex > 0) {
      for (let i = 0; i < startIndex; i++) {
        if (matchesTypeahead(filteredEntries[i].name, query)) return i;
      }
    }
    return -1;
  }

  function findMatchingPlaylistIndex(query: string, startIndex = 0): number {
    if (!query || playlistItems.length === 0) return -1;
    for (let i = startIndex; i < playlistItems.length; i++) {
      if (matchesTypeahead(playlistItems[i].name, query)) return i;
    }
    if (startIndex > 0) {
      for (let i = 0; i < startIndex; i++) {
        if (matchesTypeahead(playlistItems[i].name, query)) return i;
      }
    }
    return -1;
  }

  function performTypeToJump(query: string, advanceIfSameChar = false) {
    if (!query) return;

    if (activeTab === "playlist" && playlistItems.length > 0) {
      let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
      let startIdx = 0;
      if (advanceIfSameChar && currentIdx >= 0) {
        startIdx = currentIdx + 1;
      }
      const matchIdx = findMatchingPlaylistIndex(query, startIdx);
      if (matchIdx !== -1) {
        selectedPlaylistIndex = matchIdx;
        scrollSelectedPlaylistItemIntoView();
      }
      return;
    }

    if (activeTab === "browser" && filteredEntries.length > 0) {
      let currentIdx = filteredEntries.findIndex(e => selectedFilePaths.has(e.path));
      let startIdx = 0;
      if (advanceIfSameChar && currentIdx >= 0) {
        startIdx = currentIdx + 1;
      }
      const matchIdx = findMatchingBrowserIndex(query, startIdx);
      if (matchIdx !== -1) {
        const match = filteredEntries[matchIdx];
        selectedFilePaths.clear();
        selectedFilePaths.add(match.path);
        selectedFilePaths = selectedFilePaths;
        lastSelectedEntry = { name: match.name, path: match.path };
        scrollSelectedBrowserItemIntoView();
      }
    }
  }

  function handleTypeaheadKey(char: string) {
    const isFirstChar = typeToJumpBuffer.length === 0;

    if (isFirstChar) {
      typeToJumpBuffer = char;
      showTypeaheadIndicator = true;
      // Immediate jump on first letter!
      performTypeToJump(typeToJumpBuffer);

      if (typeToJumpTimeout) clearTimeout(typeToJumpTimeout);
      typeToJumpTimeout = setTimeout(() => {
        onTypeaheadRestPeriodElapsed();
      }, TYPEAHEAD_REST_PERIOD_MS);
    } else {
      // Within rest window:
      if (typeToJumpTimeout) clearTimeout(typeToJumpTimeout);

      // Check if user is repeating the same character (e.g. "B" then "B")
      const isRepeatedSameChar = typeToJumpBuffer.split("").every(c => c.toLowerCase() === char.toLowerCase());

      if (isRepeatedSameChar) {
        const testBuffer = typeToJumpBuffer + char;
        const hasMatchForRepeated = (activeTab === "playlist")
          ? findMatchingPlaylistIndex(testBuffer) !== -1
          : findMatchingBrowserIndex(testBuffer) !== -1;

        if (hasMatchForRepeated) {
          typeToJumpBuffer = testBuffer;
          performTypeToJump(typeToJumpBuffer);
        } else {
          // Cycle to next song starting with that letter!
          performTypeToJump(char, true);
        }
      } else {
        // Multi-character input: e.g. "B" -> "BE" immediately jumps to first "BE" song!
        typeToJumpBuffer += char;
        performTypeToJump(typeToJumpBuffer);
      }

      // Reset the window timer so once the window passes, the next key starts fresh
      typeToJumpTimeout = setTimeout(() => {
        onTypeaheadRestPeriodElapsed();
      }, TYPEAHEAD_REST_PERIOD_MS);
    }
  }

  function onTypeaheadRestPeriodElapsed() {
    // Window passed: clear buffer so subsequent typing starts fresh
    typeToJumpBuffer = "";
    showTypeaheadIndicator = false;
    if (typeToJumpTimeout) {
      clearTimeout(typeToJumpTimeout);
      typeToJumpTimeout = null;
    }
  }

  function clearTypeahead() {
    if (typeToJumpTimeout) {
      clearTimeout(typeToJumpTimeout);
      typeToJumpTimeout = null;
    }
    typeToJumpBuffer = "";
    showTypeaheadIndicator = false;
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
    if (entry.is_dir) {
      selectedFilePaths.clear();
      selectedFilePaths.add(entry.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: entry.name, path: entry.path, is_dir: true };
      return;
    }

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
      lastSelectedEntry = { name: entry.name, path: entry.path, is_dir: false };
    } else {
      // Regular click replaces selection
      selectedFilePaths.clear();
      selectedFilePaths.add(entry.path);
      selectedFilePaths = selectedFilePaths;
      lastSelectedEntry = { name: entry.name, path: entry.path, is_dir: false };
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

  let draggedPlaylistIndex: number | null = null;
  let dragOverPlaylistIndex: number | null = null;

  function handlePlaylistDragStart(e: DragEvent, idx: number) {
    draggedPlaylistIndex = idx;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", `${idx}`);
    }
  }

  function handlePlaylistDragOver(e: DragEvent, idx: number) {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = "move";
    }
    dragOverPlaylistIndex = idx;
  }

  function handlePlaylistDrop(e: DragEvent, targetIdx: number) {
    e.preventDefault();
    if (draggedPlaylistIndex !== null && draggedPlaylistIndex !== targetIdx) {
      movePlaylistItem(draggedPlaylistIndex, targetIdx);
    }
    draggedPlaylistIndex = null;
    dragOverPlaylistIndex = null;
  }

  function handlePlaylistDragEnd() {
    draggedPlaylistIndex = null;
    dragOverPlaylistIndex = null;
  }

  function movePlaylistItem(fromIdx: number, toIdx: number) {
    if (fromIdx < 0 || fromIdx >= playlistItems.length || toIdx < 0 || toIdx >= playlistItems.length) return;
    const item = playlistItems[fromIdx];
    const updated = [...playlistItems];
    updated.splice(fromIdx, 1);
    updated.splice(toIdx, 0, item);
    playlistItems = updated;
    selectedPlaylistIndex = toIdx;
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
  }

  function removePlaylistItem(idx: number) {
    playlistItems = playlistItems.filter((_, i) => i !== idx);
    if (selectedPlaylistIndex === idx) {
      selectedPlaylistIndex = -1;
    } else if (selectedPlaylistIndex > idx) {
      selectedPlaylistIndex -= 1;
    }
    localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
  }

  function clearPlaylist() {
    playlistItems = [];
    selectedPlaylistIndex = -1;
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
          bakeEnvelope: exportBakeEnvelope,
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
          envelopeNodes: isEnvelopeEnabled ? volumeEnvelopeNodes.map(n => ({
            timeSeconds: n.timeSeconds,
            gainDb: n.gainDb,
            curve: n.curve
          })) : [],
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
  let lastBroadcastStateStr = "";

  function broadcastCurrentState(force: boolean = false) {
    let activeMarkerName = "";
    let maxMarkerTime = -1;
    for (let i = 0; i < markers.length; i++) {
      const m = markers[i];
      if (m.time <= currentTime + 0.15 && m.time > maxMarkerTime) {
        maxMarkerTime = m.time;
        activeMarkerName = m.name;
      }
    }

    const statePayload = JSON.stringify({
      type: "state",
      isPlaying,
      currentTime: Math.round(currentTime * 100) / 100,
      duration: Math.round(duration * 100) / 100,
      formattedTime: formatTime(currentTime),
      formattedRemaining: formatTime(Math.max(0, duration - currentTime)),
      trackName: fileName || "No Track",
      filePath: filePath || "",
      playlistIndex: selectedPlaylistIndex >= 0 ? selectedPlaylistIndex + 1 : 0,
      playlistTotal: playlistItems.length,
      currentMarker: activeMarkerName,
      pitchSemitones: pitch + (pitchCents / 100.0),
      volumeDb: dbVolume,
      speed,
      isLooping: regions.some(r => r.isLoop && currentTime >= r.startTime && currentTime <= r.endTime)
    });

    if (!force && statePayload === lastBroadcastStateStr) {
      return;
    }
    lastBroadcastStateStr = statePayload;
    invoke("broadcast_remote_state", { stateJson: statePayload }).catch(() => {});
  }

  async function handleRemoteControlAction(payloadStr: string) {
    try {
      const parsed = typeof payloadStr === "string" ? JSON.parse(payloadStr) : payloadStr;
      const action = parsed.action || parsed.command;
      const data = parsed.data || {};

      switch (action) {
        case "get_state":
          broadcastCurrentState();
          break;
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
            }
          } else if (playlistItems.length > 0) {
            let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.min(playlistItems.length - 1, currentIdx + 1);
            selectedPlaylistIndex = currentIdx;
            scrollSelectedPlaylistItemIntoView();
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
            }
          } else if (playlistItems.length > 0) {
            let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
            if (currentIdx === -1) currentIdx = 0;
            else currentIdx = Math.max(0, currentIdx - 1);
            selectedPlaylistIndex = currentIdx;
            scrollSelectedPlaylistItemIntoView();
          }
          break;
        case "select_track":
          if (typeof data.index === "number" && data.index >= 0 && data.index < playlistItems.length) {
            selectedPlaylistIndex = data.index;
            scrollSelectedPlaylistItemIntoView();
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
          dbVolume = Math.min(12.0, parseFloat((dbVolume + 1.0).toFixed(1)));
          volumeLinear = dbToLinear(dbVolume);
          await invoke("set_volume", { volume: volumeLinear });
          saveCurrentTrackProfile(filePath);
          break;
        case "volume_down":
          dbVolume = Math.max(-60.0, parseFloat((dbVolume - 1.0).toFixed(1)));
          volumeLinear = dbToLinear(dbVolume);
          await invoke("set_volume", { volume: volumeLinear });
          saveCurrentTrackProfile(filePath);
          break;
        case "adjust_volume":
          if (typeof data.delta === "number") {
            dbVolume = Math.max(-60.0, Math.min(12.0, parseFloat((dbVolume + data.delta).toFixed(1))));
            volumeLinear = dbToLinear(dbVolume);
            await invoke("set_volume", { volume: volumeLinear });
            saveCurrentTrackProfile(filePath);
          }
          break;
        case "set_volume":
          if (typeof data.db === "number") {
            dbVolume = Math.max(-60.0, Math.min(12.0, parseFloat(data.db.toFixed(1))));
            volumeLinear = dbToLinear(dbVolume);
            await invoke("set_volume", { volume: volumeLinear });
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
        case "load_playlist":
          if (Array.isArray(data.items)) {
            playlistItems = data.items;
            localStorage.setItem("th_playlist", JSON.stringify(playlistItems));
            if (playlistItems.length > 0 && playlistItems[0].path && !playlistItems[0].isPlaceholder) {
              loadAudioPath(playlistItems[0].path, "main", true);
            }
            checkPlaylistHealthStatus();
          }
          break;
        case "open_setlist_importer":
          showSetlistModal = true;
          break;
        case "open_repair_modal":
          openRepairModal();
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
          dbVolume = parseFloat((-60 + norm * 66).toFixed(1));
          volumeLinear = dbToLinear(dbVolume);
          invoke("set_volume", { volume: volumeLinear });
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
    if (!mainCanvas || isPanning || isShiftSelecting || isDraggingMarker || isDraggingSelectionEdge || isDraggingRegionEdge || isDraggingEnvelopeNode) return;
    const rect = mainCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const clickY = e.clientY - rect.top;
    const windowWidth = 1.0 / zoom;
    const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
    const startTime = startProgress * duration;
    const visibleSec = windowWidth * duration;

    // Check envelope node hover
    if (isEnvelopeOverlayVisible && volumeEnvelopeNodes.length > 0) {
      const envTopY = 30;
      const envBotY = rect.height - 10;
      const envRangeDb = 72.0;
      const dbToY = (db: number) => envTopY + ((12.0 - Math.max(-60, Math.min(12, db))) / envRangeDb) * (envBotY - envTopY);

      let foundNodeId: string | null = null;
      for (const node of volumeEnvelopeNodes) {
        const nx = ((node.timeSeconds - startTime) / visibleSec) * rect.width;
        const ny = dbToY(node.gainDb);
        if (Math.hypot(clickX - nx, clickY - ny) <= 10) {
          foundNodeId = node.id;
          break;
        }
      }
      if (foundNodeId !== hoveredEnvelopeNodeId) {
        hoveredEnvelopeNodeId = foundNodeId;
        drawMainWaveform();
      }
      if (foundNodeId) {
        mainCanvas.style.cursor = "pointer";
        return;
      }
    }

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
    const startTime = startProgress * duration;
    const visibleSec = windowWidth * duration;

    // Check if clicked near an envelope node
    if (isEnvelopeOverlayVisible && volumeEnvelopeNodes.length > 0 && clickY > 20) {
      const envTopY = 30;
      const envBotY = rect.height - 10;
      const envRangeDb = 72.0;
      const dbToY = (db: number) => envTopY + ((12.0 - Math.max(-60, Math.min(12, db))) / envRangeDb) * (envBotY - envTopY);

      for (const node of volumeEnvelopeNodes) {
        const nx = ((node.timeSeconds - startTime) / visibleSec) * rect.width;
        const ny = dbToY(node.gainDb);
        if (Math.hypot(clickX - nx, clickY - ny) <= 12) {
          selectedEnvelopeNodeId = node.id;
          isDraggingEnvelopeNode = true;
          draggedEnvelopeNodeId = node.id;
          window.addEventListener("mousemove", handleMainMouseMove);
          window.addEventListener("mouseup", handleMainMouseUp);
          drawMainWaveform();
          return;
        }
      }
    }

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

    // Normal click/pan: clear time selection and deselect envelope nodes if clicking plain waveform
    timeSelection = null;
    if (selectedEnvelopeNodeId) {
      selectedEnvelopeNodeId = null;
      drawMainWaveform();
    }
    
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

    if (isDraggingEnvelopeNode && draggedEnvelopeNodeId && mainCanvas) {
      const rect = mainCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickY = e.clientY - rect.top;
      const windowWidth = 1.0 / zoom;
      const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
      const startTime = startProgress * duration;
      const visibleSec = windowWidth * duration;

      const envTopY = 30;
      const envBotY = rect.height - 10;
      const envRangeDb = 72.0;
      const clampedY = Math.max(envTopY, Math.min(envBotY, clickY));
      const norm = (clampedY - envTopY) / (envBotY - envTopY);
      const newGainDb = Math.max(-60, Math.min(12, 12.0 - norm * envRangeDb));
      const newTime = Math.max(0, Math.min(duration, startTime + (clickX / rect.width) * visibleSec));

      const node = volumeEnvelopeNodes.find(n => n.id === draggedEnvelopeNodeId);
      if (node) {
        node.timeSeconds = newTime;
        node.gainDb = Math.round(newGainDb * 10) / 10;
        volumeEnvelopeNodes = [...volumeEnvelopeNodes].sort((a, b) => a.timeSeconds - b.timeSeconds);
        updateVolumeEnvelopeEngine();
      }
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
    if (isDraggingEnvelopeNode) {
      isDraggingEnvelopeNode = false;
      draggedEnvelopeNodeId = null;
      window.removeEventListener("mousemove", handleMainMouseMove);
      window.removeEventListener("mouseup", handleMainMouseUp);
      updateVolumeEnvelopeEngine();
      return;
    }

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

  function handleMainDblClick(e: MouseEvent) {
    if (duration === 0 || !mainCanvas || !isEnvelopeOverlayVisible) return;
    const rect = mainCanvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const clickY = e.clientY - rect.top;
    if (clickY <= 24) return; // Ignore top ruler area

    const windowWidth = 1.0 / zoom;
    const startProgress = zoom > 1.001 ? Math.max(0, Math.min(1.0 - windowWidth, progress - windowWidth / 2)) : 0;
    const startTime = startProgress * duration;
    const visibleSec = windowWidth * duration;
    const clickTime = startTime + (clickX / rect.width) * visibleSec;

    const envTopY = 30;
    const envBotY = rect.height - 10;
    const envRangeDb = 72.0;
    const clampedY = Math.max(envTopY, Math.min(envBotY, clickY));
    const norm = (clampedY - envTopY) / (envBotY - envTopY);
    const gainDb = Math.max(-60, Math.min(12, 12.0 - norm * envRangeDb));

    addVolumeEnvelopeNodeAt(clickTime, Math.round(gainDb * 10) / 10, "linear");
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
    const activePdfCont = getActivePdfContainer();
    if (activePdfCont) {
      const match = markers.find(m => Math.abs(m.time - time) < 0.2);
      let anchor = match?.pdfAnchor;
      if (!anchor && activeTrackMode === "alternate" && mainTrack) {
        const mainMarkers = getProfilesStore()[mainTrack.path]?.markers || [];
        const mm = mainMarkers.find(m => m.name.trim().toLowerCase() === match?.name.trim().toLowerCase());
        anchor = mm?.pdfAnchor;
      }
      if (anchor) {
        const markerPage = anchor.page;
        const card = activePdfCont.querySelector(`.pdf-page-card[data-page-num="${markerPage}"]`) as HTMLElement;
        if (card) {
          const containerRect = activePdfCont.getBoundingClientRect();
          const cardRect = card.getBoundingClientRect();
          const cardTopRelativeToContainer = (cardRect.top - containerRect.top) + activePdfCont.scrollTop;
          const markerYWithinCard = card.clientHeight * anchor.yPct;
          const targetScrollTop = Math.max(0, cardTopRelativeToContainer + markerYWithinCard - 12);
          activePdfCont.scrollTo({
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
        // A. Draw original uncompressed waveform as translucent white ghost (scaled with envelope)
        ctx.strokeStyle = "rgba(255, 255, 255, 0.22)";
        ctx.lineWidth = 1.0;
        ctx.beginPath();
        for (let i = 0; i < numSamples; i++) {
          const t_i = startTime + (i / (numSamples - 1 || 1)) * visibleSec;
          const envLinear = calcEnvelopeLinearGainAt(t_i);
          const rawAmp = visibleSamples[i] * envLinear;
          const x = i * step;
          const y = halfHeight - rawAmp * maxAmplitude;
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
          const t_i = startTime + (i / (numSamples - 1 || 1)) * visibleSec;
          const envLinear = calcEnvelopeLinearGainAt(t_i);
          const rawAmp = visibleSamples[i] * envLinear;
          const absAmp = Math.abs(rawAmp);
          const sign = rawAmp >= 0 ? 1 : -1;
          
          let compAmp = absAmp;
          if (absAmp > threshLinear) {
            const rawDb = 20.0 * Math.log10(Math.max(1e-6, absAmp));
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
        // Standard normal continuous waveform scaled dynamically by volume envelope in real time!
        ctx.strokeStyle = "#3b99fc";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < numSamples; i++) {
          const t_i = startTime + (i / (numSamples - 1 || 1)) * visibleSec;
          const envLinear = calcEnvelopeLinearGainAt(t_i);
          const rawAmp = visibleSamples[i] * envLinear;
          const x = i * step;
          const y = halfHeight - rawAmp * maxAmplitude;
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
          const t_i = startTime + (i / (numSamples - 1 || 1)) * visibleSec;
          const envLinear = calcEnvelopeLinearGainAt(t_i);
          const rawAmp = visibleSamples[i] * envLinear;
          const x = i * step;
          const y = halfHeight - rawAmp * maxAmplitude;
          ctx.fillRect(x - nodeSize / 2, y - nodeSize / 2, nodeSize, nodeSize);
          if (nodeSize >= 4) {
            ctx.strokeStyle = "#0c151e";
            ctx.lineWidth = 1;
            ctx.strokeRect(x - nodeSize / 2, y - nodeSize / 2, nodeSize, nodeSize);
          }
        }
      }
    }

    // 6.5 Interactive Volume Envelope Overlay (+12 dB to -60 dB, Multi-curve nodes)
    if (isEnvelopeOverlayVisible && duration > 0) {
      const envTopY = rulerHeight + 10;
      const envBotY = height - 10;
      const envRangeDb = 72.0; // from +12dB to -60dB = 72dB range

      const dbToY = (db: number) => {
        const clamped = Math.max(-60, Math.min(12, db));
        const norm = (12.0 - clamped) / envRangeDb;
        return envTopY + norm * (envBotY - envTopY);
      };

      ctx.save();

      // Reference Grid lines for Envelope (+12, +6, 0, -6, -12, -24, -60 dB)
      const gridDbs = [12, 6, 0, -6, -12, -24, -60];
      for (const gDb of gridDbs) {
        const gy = dbToY(gDb);
        const isUnity = gDb === 0;
        ctx.strokeStyle = isUnity ? "rgba(255, 179, 64, 0.4)" : "rgba(255, 179, 64, 0.12)";
        ctx.lineWidth = isUnity ? 1.0 : 0.75;
        ctx.setLineDash(isUnity ? [4, 3] : [2, 4]);
        ctx.beginPath();
        ctx.moveTo(0, gy);
        ctx.lineTo(width, gy);
        ctx.stroke();

        ctx.fillStyle = isUnity ? "rgba(255, 179, 64, 0.9)" : "rgba(255, 179, 64, 0.4)";
        ctx.font = isUnity ? "bold 8px monospace" : "8px monospace";
        ctx.fillText(`${gDb > 0 ? "+" : ""}${gDb} dB`, width - 36, gy - 2);
      }
      ctx.setLineDash([]);

      // Stroke clean envelope curve line
      const envSteps = Math.min(400, Math.max(50, Math.floor(width)));
      ctx.beginPath();
      for (let si = 0; si <= envSteps; si++) {
        const sx = (si / envSteps) * width;
        const st = startTime + (si / envSteps) * visibleSec;
        const sDb = calcEnvelopeGainDbAt(st);
        const sy = dbToY(sDb);
        if (si === 0) ctx.moveTo(sx, sy);
        else ctx.lineTo(sx, sy);
      }
      ctx.strokeStyle = "#ffb340";
      ctx.lineWidth = 1.75;
      ctx.stroke();

      // Draw nodes
      for (const node of volumeEnvelopeNodes) {
        const nodePct = (node.timeSeconds - startTime) / visibleSec;
        if (nodePct >= -0.05 && nodePct <= 1.05) {
          const nx = nodePct * width;
          const ny = dbToY(node.gainDb);
          const isSelected = selectedEnvelopeNodeId === node.id;
          const isHovered = hoveredEnvelopeNodeId === node.id;
          const isDraggingThis = isDraggingEnvelopeNode && draggedEnvelopeNodeId === node.id;

          ctx.fillStyle = isSelected ? "#ffffff" : (isHovered ? "#ffe066" : "#ffb340");
          ctx.beginPath();
          ctx.arc(nx, ny, isSelected ? 6 : 4.5, 0, Math.PI * 2);
          ctx.fill();

          ctx.strokeStyle = isSelected ? "#ffb340" : "#08080a";
          ctx.lineWidth = isSelected ? 2.0 : 1.2;
          ctx.stroke();

          // Only show floating badge on hover or during active drag to avoid clutter
          if (isHovered || isDraggingThis) {
            ctx.fillStyle = "rgba(18, 20, 26, 0.95)";
            ctx.fillRect(nx + 8, ny - 16, 92, 20);
            ctx.strokeStyle = "#ffb340";
            ctx.lineWidth = 1;
            ctx.strokeRect(nx + 8, ny - 16, 92, 20);

            ctx.fillStyle = "#ffffff";
            ctx.font = "bold 9px monospace";
            ctx.fillText(`${node.gainDb > 0 ? "+" : ""}${node.gainDb.toFixed(1)}dB [${node.curve.toUpperCase()}]`, nx + 12, ny - 3);
          }
        }
      }
      ctx.restore();
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

  function handleContainerMousedown(e: MouseEvent) {
    const target = e.target as HTMLElement | null;
    const inSidebar = target?.closest(".sidebar-left");
    if (inSidebar) {
      lastFocusedPane = "sidebar";
    } else {
      lastFocusedPane = "waveform";
      clearTypeahead();
    }
  }
</script>

<main class="app-container">
  <div 
    class="workspace-grid" 
    class:resizing-active={isResizingLeft || isResizingRight}
    style="grid-template-columns: {leftSidebarWidth}px 4px 1fr 4px {rightSidebarWidth}px;"
  >
    
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
          <span class="current-dir-label" title={currentPath}>{getFolderDisplayName(currentPath)}</span>
          {#if parentPath}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <span class="up-btn" on:click={() => loadBrowser(parentPath)}>Parent ↰</span>
          {/if}
        </div>

        <div class="browser-quick-bar">
          <button 
            class="quick-jump-btn" 
            class:active={currentPath === rootName}
            on:click={() => loadBrowser(rootName)}
            title="Browse all drives and storage ({rootName})"
          >
            💻 {rootName}
          </button>

          <button 
            class="quick-jump-btn" 
            class:active={isCurrentPathHome}
            on:click={() => loadBrowser("~")}
            title="Go to User Home directory"
          >
            🏠 Home
          </button>

          {#if cloudFolders && cloudFolders.length > 0}
            <div class="cloud-btn-wrapper">
              <button 
                class="quick-jump-btn cloud-jump-btn" 
                class:active={isCurrentPathCloud}
                on:click={handleCloudJumpClick}
                title="Jump to Cloud Storage"
              >
                ☁️ Cloud {cloudFolders.length > 1 ? "▾" : ""}
              </button>

              {#if showCloudDropdown}
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <!-- svelte-ignore a11y-no-static-element-interactions -->
                <div class="cloud-dropdown-menu" on:click|stopPropagation>
                  {#each cloudFolders as folder}
                    <button 
                      class="cloud-dropdown-item" 
                      class:active={currentPath.toLowerCase().startsWith(folder.path.toLowerCase())}
                      on:click={() => {
                        showCloudDropdown = false;
                        loadBrowser(folder.path);
                      }}
                      title={folder.path}
                    >
                      <span class="cloud-item-icon">☁️</span>
                      <span class="cloud-item-name">{folder.name}</span>
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
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
                  if (filePath && associatedFiles.some(f => f.path === entry.path)) {
                    loadAudioVersion(entry.path, true);
                  } else {
                    loadAudioPath(entry.path, "main");
                  }
                }
              }}
            >
              <span class="item-icon">
                {entry.kind === "drive" || entry.name.startsWith("Drive (") || entry.name.includes(" (Volume)") ? "💾" : (entry.kind === "cloud" || entry.name.includes("Dropbox") || entry.name.includes("OneDrive") || entry.name.includes("Google Drive") ? "☁️" : (entry.is_dir ? "📁" : "🎵"))}
              </span>
              <span class="item-name" title={entry.name}>{entry.name}</span>
              {#if !entry.is_dir && isAudioFile(entry.name || entry.path)}
                <div class="browser-item-tags">
                  {#each getFileTags(entry.path, entry.name).slice(0, 2) as tag}
                    {#if tag === "Original"}
                      <span class="browser-mini-tag mini-orig" title="Original Reference">ORIG</span>
                    {:else if tag === "Vocals only"}
                      <span class="browser-mini-tag mini-vocals" title="Vocals Only">VOCALS</span>
                    {:else if tag === "Lead Vocal"}
                      <span class="browser-mini-tag mini-lead" title="Lead Vocal">LEAD</span>
                    {:else if tag === "Background Vocals"}
                      <span class="browser-mini-tag mini-backings" title="Background Vocals">BGV</span>
                    {:else if tag === "Iso Track"}
                      <span class="browser-mini-tag mini-iso" title="Iso Track">ISO</span>
                    {:else if tag === "Track"}
                      <span class="browser-mini-tag mini-track" title="Track / Accompaniment">TRACK</span>
                    {/if}
                  {/each}
                </div>
              {/if}
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
                class:dragging={draggedPlaylistIndex === idx}
                class:drag-over={dragOverPlaylistIndex === idx}
                class:is-unlinked={item.isPlaceholder || item.missingAudio}
                draggable="true"
                on:dragstart={(e) => handlePlaylistDragStart(e, idx)}
                on:dragover={(e) => handlePlaylistDragOver(e, idx)}
                on:dragleave={() => { if (dragOverPlaylistIndex === idx) dragOverPlaylistIndex = null; }}
                on:drop={(e) => handlePlaylistDrop(e, idx)}
                on:dragend={handlePlaylistDragEnd}
                on:click={() => { selectedPlaylistIndex = idx; }}
                on:dblclick={() => {
                  selectedPlaylistIndex = idx;
                  if (item.path && !item.isPlaceholder && !item.missingAudio) {
                    loadAudioPath(item.path, "main", true).then(async () => {
                      await invoke("play");
                      isPlaying = true;
                    });
                  } else {
                    openRepairModal();
                  }
                }}
              >
                <span class="playlist-drag-handle" title="Drag to reorder">⋮⋮</span>
                <span class="item-icon">{item.isPlaceholder || item.missingAudio ? "⚠️" : "🎵"}</span>
                <div class="playlist-item-text-stack">
                  <div class="playlist-item-title-row">
                    <span class="item-name" title={item.name}>{item.name}</span>
                    {#if item.isPlaceholder || item.missingAudio}
                      <span class="unlinked-pill" title="Audio file missing or unlinked">Unlinked</span>
                    {/if}
                  </div>
                </div>
                <div class="playlist-item-actions">
                  <button class="reorder-item-btn" disabled={idx === 0} on:click|stopPropagation={() => movePlaylistItem(idx, idx - 1)} title="Move Up (Option+Up)">▲</button>
                  <button class="reorder-item-btn" disabled={idx === playlistItems.length - 1} on:click|stopPropagation={() => movePlaylistItem(idx, idx + 1)} title="Move Down (Option+Down)">▼</button>
                  <button class="remove-playlist-item-btn" on:click|stopPropagation={() => removePlaylistItem(idx)} title="Remove track">×</button>
                </div>
              </div>
            {/each}
          {/if}
        </div>

        <div class="playlist-controls-sidebar">
          <button class="action-btn setlist-ai-btn" on:click={openSetlistModal} title="Import Setlist from CSV table or clipboard">
            🪄 Setlist...
          </button>
          <button class="action-btn repair-btn" on:click={openRepairModal} title="Verify playlist health and repair missing files / upgraded mixes">
            🔄 Repair {#if missingTracksCount > 0}<span class="missing-count-badge">({missingTracksCount})</span>{/if}
          </button>
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

      <!-- Typeahead Jump Indicator Overlay -->
      {#if showTypeaheadIndicator && typeToJumpBuffer}
        <div class="typeahead-indicator">
          <span class="typeahead-icon">🔤</span>
          <span class="typeahead-text">Jump: <strong>{typeToJumpBuffer}</strong></span>
        </div>
      {/if}
    </aside>

    <!-- LEFT SIDEBAR RESIZER HANDLE -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div 
      class="resizer-handle resizer-left" 
      class:active={isResizingLeft} 
      on:mousedown={startResizeLeft} 
      title="Drag to resize Left Sidebar"
    ></div>

    <!-- CENTER AREA: Resizable Waveforms & Fixed Bottom controls -->
    <section class="center-content" bind:this={centerContentElement}>
      
      <!-- Current File info header (Single compact row) -->
      <div class="track-header">
        <div class="track-title-info">
          <span class="track-badge" class:main-badge={filePath === primarySongTrackPath || !primarySongTrackPath}>
            {getTrackWatermarkLabel()}
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
            FILES {#if allSongFiles.length > 0}<span class="files-badge-count">({allSongFiles.length})</span>{/if}
            {#if uvrSeparating}
              <span class="uvr-tab-indicator" title="UVR Separation in progress">✨ {uvrPercent}%</span>
            {/if}
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
                  <button class="md-toggle-btn" class:active={notesViewMode === 'edit'} on:click={() => notesViewMode = 'edit'} title="View and edit raw Markdown text">✏️ Raw Text</button>
                  <button class="md-toggle-btn" class:active={notesViewMode === 'preview'} on:click={() => notesViewMode = 'preview'} title="View rendered rich Markdown">👁️ Rendered</button>
                  <button class="md-toggle-btn" class:active={notesViewMode === 'split'} on:click={() => notesViewMode = 'split'} title="Split side-by-side view">◫ Split</button>
                </div>
              </div>

              <div class="markdown-editor-container mode-{notesViewMode}">
                {#if notesViewMode === 'edit' || notesViewMode === 'split'}
                  <textarea 
                    class="rehearsal-textarea md-textarea" 
                    placeholder="Type rehearsal notes, singer assignments (tables e.g. | Song | Singer |), chord cues (e.g. [C#m7]), key changes, or arrangement details using Markdown (auto-saved)..."
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
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'edit'} on:click={() => lyricsViewMode = 'edit'} title="View and edit raw Markdown text">✏️ Raw Text</button>
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'preview'} on:click={() => lyricsViewMode = 'preview'} title="View rendered rich Markdown">👁️ Rendered</button>
                  <button class="md-toggle-btn" class:active={lyricsViewMode === 'split'} on:click={() => lyricsViewMode = 'split'} title="Split side-by-side view">◫ Split</button>
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
                  <span class="files-pane-title">
                    ASSOCIATED FILES
                    {#if allSongFiles.length > 0}
                      <span class="files-count-pill">{allSongFiles.length}</span>
                    {/if}
                  </span>
                </div>
                <div class="files-header-right">
                  <button 
                    class="prefer-hires-btn" 
                    class:active={preferHighResAudio}
                    on:click={togglePreferHighResAudio}
                    title="When enabled, TrackHelm will automatically load the high-res lossless audio file as main track IF it is downloaded locally on your Mac"
                  >
                    ⚡ {preferHighResAudio ? "High-Res Preferred (Downloaded)" : "Prefer High-Res Audio"}
                  </button>
                  <button class="add-assoc-file-btn" on:click={addAssociatedFilePicker}>
                    + Add Associated File...
                  </button>
                </div>
              </div>


              <!-- UVR Separation Status Banner (Slim single-row) -->
              {#if uvrSeparating}
                <div class="uvr-status-bar">
                  <span class="uvr-sparkle">✨</span>
                  <span class="uvr-stage">{uvrStage}</span>
                  {#if uvrCurrentTarget}
                    <span class="uvr-target" title={uvrCurrentTarget}>({uvrCurrentTarget})</span>
                  {/if}
                  <div class="uvr-meter-track">
                    <div class="uvr-meter-fill" style="width: {uvrPercent}%"></div>
                  </div>
                  <span class="uvr-pct">{uvrPercent}%</span>
                </div>
              {/if}

              {#if uvrErrorMessage}
                <div class="uvr-toast uvr-toast-error">
                  <span>⚠️ {uvrErrorMessage}</span>
                  <button class="uvr-dismiss-btn" on:click={() => uvrErrorMessage = null}>×</button>
                </div>
              {/if}

              {#if uvrSuccessMessage}
                <div class="uvr-toast uvr-toast-success">
                  <span>✨ {uvrSuccessMessage}</span>
                  <button class="uvr-dismiss-btn" on:click={() => uvrSuccessMessage = null}>×</button>
                </div>
              {/if}

              <div class="associated-files-list">
                {#if allSongFiles.length === 0}
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
                  <div class="assoc-files-table">
                    {#each allSongFiles as item (item.id || item.path)}
                      <!-- svelte-ignore a11y-no-static-element-interactions -->
                      <div 
                        class="assoc-file-row" 
                        class:is-active-waveform={item.path === filePath}
                        on:dblclick={() => {
                          if (item.fileType === 'audio') {
                            loadAssociatedTrackIntoActive(item);
                          } else if (item.fileType === 'pdf') {
                            openPdfTab(item.path, item.name);
                          }
                        }}
                        on:contextmenu={(e) => handleAssocFileContextMenu(e, item)}
                        title="{item.name}&#10;Double-click to load into waveform • Right-click for options&#10;Path: {item.path}"
                      >
                        <!-- Left Side: Icon + Name + Badges -->
                        <div class="assoc-row-left">
                          <span class="assoc-row-icon">
                            {#if item.fileType === 'pdf'}
                              📄
                            {:else if item.role === 'lead' || item.name.toLowerCase().includes('lead vocal')}
                              🎤
                            {:else if item.role === 'backings' || item.name.toLowerCase().includes('backing vocal')}
                              👥
                            {:else if item.role === 'vocals' || item.name.toLowerCase().includes('vocals ensemble') || item.name.toLowerCase().includes('(vocals)')}
                              ✨
                            {:else if isLosslessAudio(item.path || item.name)}
                              🎼
                            {:else}
                              🎵
                            {/if}
                          </span>

                          <span class="assoc-row-name" title={item.path}>{item.name}</span>

                          <div class="assoc-row-badges">
                            <span 
                              class="assoc-ext-badge" 
                              class:badge-pdf={item.fileType === 'pdf'} 
                              class:badge-lossless={item.fileType === 'audio' && isLosslessAudio(item.path || item.name)} 
                              class:badge-lossy={item.fileType === 'audio' && !isLosslessAudio(item.path || item.name)}
                            >
                              {getFileExtension(item.path || item.name) || (item.fileType === 'pdf' ? 'PDF' : 'AUDIO')}
                            </span>

                            {#if item.path === filePath}
                              <span class="assoc-role-badge badge-active-waveform" title="Currently loaded in the active waveform display">● ACTIVE</span>
                            {/if}

                            {#if item.path === primarySongTrackPath}
                              <span class="assoc-role-badge badge-default-mix" title="Primary default track for this song">DEFAULT</span>
                            {/if}

                            {#if item.fileType === 'audio'}
                              {#each getFileTags(item.path, item.name, item.role) as tag}
                                {#if tag === "Original"}
                                  <span class="assoc-role-badge badge-orig" title="Original Artist Reference Recording (Right-click to toggle)">ORIGINAL</span>
                                {:else if tag === "Vocals only"}
                                  <span class="assoc-role-badge badge-vocals" title="Master Isolated Vocals (Right-click to toggle)">VOCALS</span>
                                {:else if tag === "Lead Vocal"}
                                  <span class="assoc-role-badge badge-lead" title="Lead Vocal Track (Right-click to toggle)">LEAD</span>
                                {:else if tag === "Background Vocals"}
                                  <span class="assoc-role-badge badge-backings" title="Background Vocals Track (Right-click to toggle)">BACKINGS</span>
                                {:else if tag === "Iso Track"}
                                  <span class="assoc-role-badge badge-iso-track" title="Isolated Instrumental Track from Vocal Separation (Right-click to toggle)">ISO TRACK</span>
                                {:else if tag === "Track"}
                                  <span class="assoc-role-badge badge-track" title="Accompaniment / Backing Track (Right-click to toggle)">TRACK</span>
                                {:else}
                                  <span class="assoc-role-badge" title="Audio Tag: {tag}">{tag.toUpperCase()}</span>
                                {/if}
                              {/each}
                              {#if item.id === 'audio-hires' || isLosslessAudio(item.path || item.name)}
                                <span class="assoc-role-badge badge-hires" title="Full Resolution Lossless Master Audio">FULL-RES</span>
                              {/if}
                            {/if}
                          </div>
                        </div>

                        <!-- Right Side: Action Buttons -->
                        <div class="assoc-row-actions">
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
                              class:active-loaded={item.path === filePath}
                              disabled={item.path === filePath}
                              on:click={() => loadAudioVersion(item.path)}
                              title={item.path === filePath ? "Currently loaded in waveform" : "Load into waveform display"}
                            >
                              {item.path === filePath ? "✓ Loaded" : "▶ Load"}
                            </button>

                            <!-- UVR Stem Separation Actions -->
                            {#if !item.role?.includes('lead') && !item.role?.includes('backing') && !(item.name && (item.name.toLowerCase().includes('lead') || item.name.toLowerCase().includes('backing') || item.name.toLowerCase().includes('bgv')))}
                              {#if item.role === 'vocals' || (item.name && (item.name.toLowerCase().includes('vocals') || item.name.toLowerCase().includes('ensemble')))}
                                <button 
                                  class="assoc-action-btn uvr-action-btn uvr-karaoke-btn" 
                                  disabled={uvrSeparating}
                                  on:click={() => triggerUvrSeparation('lead_backups', item.path)}
                                  title="Split Vocals into Lead & Backing Tracks using 5HP Karaoke"
                                >
                                  🎤 Lead/Backups
                                </button>
                              {:else}
                                <button 
                                  class="assoc-action-btn uvr-action-btn uvr-ensemble-btn" 
                                  disabled={uvrSeparating}
                                  on:click={() => triggerUvrSeparation('ensemble_vocals', item.path)}
                                  title="Run 4-Model Ensemble to Isolate Vocals to AAC"
                                >
                                  ✨ Isolate Vocals
                                </button>
                              {/if}
                            {/if}
                          {/if}

                          <button 
                            class="assoc-action-btn trash-action-btn" 
                            on:click={() => requestMoveToTrash(item)}
                            title="Move file to macOS Trash"
                          >
                            🗑️
                          </button>
                          {#if item.path !== filePath && item.path !== primarySongTrackPath}
                            <button 
                              class="assoc-action-btn unlink-action" 
                              on:click={() => unlinkAssociatedFile(item.id)}
                              title="Unlink file from this project"
                            >
                              ×
                            </button>
                          {/if}
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
                      <span class="spec-label">Audio Version:</span>
                      <span class="spec-val active-slot">{getTrackWatermarkLabel()}</span>
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
        <div class="waveform-block block-overview" title="Right-click for audio versions & A/B options">
          <span 
            class="overview-watermark-tag" 
            class:alt-tag={filePath !== primarySongTrackPath && primarySongTrackPath} 
            class:main-tag={filePath === primarySongTrackPath || !primarySongTrackPath}
          >
            {getTrackWatermarkLabel()}
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

        <!-- Waveform Height Resize Handle (Between Overview & Main Waveform) -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="waveform-resize-handle" 
          class:is-resizing={isResizingWaveform}
          on:mousedown={startWaveformResize}
          on:dblclick={resetWaveformHeight}
          title="Drag up or down to adjust waveform height (Double-click to reset)"
        >
          <div class="resize-handle-line"></div>
          <div class="resize-handle-grip"></div>
          <div class="resize-handle-line"></div>
        </div>

        <!-- Main Waveform Box (Height adjustable) -->
        <div 
          class="waveform-block block-main-waveform" 
          style="height: {waveformHeight}px; min-height: {waveformHeight}px; max-height: {waveformHeight}px;"
          title="Right-click for audio versions & A/B options • Drop marker to place"
          on:dragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; }}
          on:drop={handleWaveformMarkerDrop}
        >
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <canvas 
            bind:this={mainCanvas} 
            on:mousedown={handleMainMouseDown}
            on:mousemove={handleMainCanvasHover}
            on:dblclick={handleMainDblClick}
            on:wheel|passive={handleMainWheel}
            on:contextmenu={handleWaveformContextMenu}
            class="main-canvas"
          ></canvas>
        </div>
      </div>

      <!-- Volume Envelope Strip (Dorico Theme) -->
      <div class="envelope-strip">
        <div class="envelope-strip-left">
          <button 
            class="envelope-toggle-btn" 
            class:active={isEnvelopeOverlayVisible} 
            on:click={() => { isEnvelopeOverlayVisible = !isEnvelopeOverlayVisible; drawMainWaveform(); }}
            title="Toggle Volume Envelope overlay lane on waveform"
          >
            📈 Volume Envelope
          </button>
          
          <button 
            class="envelope-state-btn" 
            class:bypassed={!isEnvelopeEnabled} 
            on:click={() => { isEnvelopeEnabled = !isEnvelopeEnabled; updateVolumeEnvelopeEngine(); }}
            title={isEnvelopeEnabled ? "Volume Envelope Active (Click to Bypass)" : "Volume Envelope Bypassed (Click to Enable)"}
          >
            {isEnvelopeEnabled ? "ACTIVE" : "BYPASSED"}
          </button>

          <button 
            class="envelope-reset-btn" 
            on:click={resetVolumeEnvelope}
            disabled={volumeEnvelopeNodes.length === 0}
            title="Reset volume envelope back to flat 0 dB (removes all nodes)"
          >
            ↺ Reset Flat
          </button>

          <span class="envelope-node-count-badge">
            {volumeEnvelopeNodes.length} node{volumeEnvelopeNodes.length === 1 ? '' : 's'}
          </span>
        </div>

        <!-- Selected Node Inspector Strip -->
        <div class="envelope-strip-right">
          {#if selectedEnvelopeNodeId && volumeEnvelopeNodes.find(n => n.id === selectedEnvelopeNodeId)}
            {@const selNode = volumeEnvelopeNodes.find(n => n.id === selectedEnvelopeNodeId)}
            {#if selNode}
              <div class="envelope-node-editor">
                <span class="env-editor-label">Node:</span>
                <span class="env-time-label">⏱ {formatTime(selNode.timeSeconds)}</span>
                
                <div class="env-gain-control">
                  <span class="env-gain-label">{selNode.gainDb > 0 ? '+' : ''}{selNode.gainDb.toFixed(1)} dB</span>
                  <input 
                    type="range" 
                    min="-60" 
                    max="12" 
                    step="0.5" 
                    value={selNode.gainDb} 
                    on:input={(e) => updateEnvelopeNodeGain(selNode.id, parseFloat(e.currentTarget.value))} 
                    class="env-gain-slider"
                    title="Adjust node gain (-60 dB to +12 dB)"
                  />
                </div>

                <!-- Curve Type Selector -->
                <div class="env-curve-selector">
                  <button 
                    class="env-curve-btn" 
                    class:active={selNode.curve === 'linear'} 
                    on:click={() => setEnvelopeNodeCurve(selNode.id, 'linear')}
                    title="Straight / Linear interpolation"
                  >
                    Straight
                  </button>
                  <button 
                    class="env-curve-btn" 
                    class:active={selNode.curve === 'curve_up'} 
                    on:click={() => setEnvelopeNodeCurve(selNode.id, 'curve_up')}
                    title="Curve Up / Exponential ramp"
                  >
                    Curve Up
                  </button>
                  <button 
                    class="env-curve-btn" 
                    class:active={selNode.curve === 'curve_down'} 
                    on:click={() => setEnvelopeNodeCurve(selNode.id, 'curve_down')}
                    title="Curve Down / Logarithmic ramp"
                  >
                    Curve Down
                  </button>
                  <button 
                    class="env-curve-btn" 
                    class:active={selNode.curve === 'sine'} 
                    on:click={() => setEnvelopeNodeCurve(selNode.id, 'sine')}
                    title="Sine / S-Curve Ease In & Out"
                  >
                    Sine S-Curve
                  </button>
                </div>

                <button 
                  class="env-delete-node-btn" 
                  on:click={() => removeVolumeEnvelopeNode(selNode.id)}
                  title="Delete this envelope node (or press Delete key)"
                >
                  🗑
                </button>
              </div>
            {/if}
          {:else}
            <span class="env-hint-text">Double-click on waveform to add an envelope node • Max +12 dB (Pre-Dynamics)</span>
          {/if}
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
            <button 
              class="control-btn ab-compare-btn" 
              class:ab-active={Boolean(primarySongTrackPath && filePath !== primarySongTrackPath)} 
              on:click={triggerAbCompare} 
              title="A/B Compare: Switch between default mix and selected stem (T)"
            >
              ⇄ A/B
            </button>
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

    <!-- RIGHT SIDEBAR RESIZER HANDLE -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div 
      class="resizer-handle resizer-right" 
      class:active={isResizingRight} 
      on:mousedown={startResizeRight} 
      title="Drag to resize Right Sidebar"
    ></div>

    <!-- RIGHT SIDEBAR: Markers & Regions, Effects Rack (Dorico Theme) -->
    <aside class="sidebar-right">
      
      <!-- Markers & Regions List (Dedicated full vertical height) -->
      <div class="panel-section markers-section">
        <div class="panel-header">
          MARKERS & REGIONS
          <span class="marker-track-mode-badge" class:alt-badge={filePath !== primarySongTrackPath && primarySongTrackPath}>
            ({getTrackWatermarkLabel()})
          </span>
        </div>
        <div class="markers-list">
          {#if markers.length === 0}
            <div class="placeholder-text">
              No markers set. Click "+ Add Marker" (or press M) during playback.
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
                      on:click|stopPropagation={() => openXfadeModal(region)}
                    >
                      ✕ {region.crossfadeMs ?? 5}ms
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
                    ✕
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
        <!-- Waveform Context Menu: Audio Versions & A/B Compare -->
        <div class="menu-section-header">Audio Versions</div>

        <!-- Primary Default Track (if not already listed in associatedFiles) -->
        {#if primarySongTrackPath && !associatedFiles.some(f => f.path === primarySongTrackPath)}
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div 
            class="menu-item menu-item-assoc" 
            class:menu-active={filePath === primarySongTrackPath}
            on:click={() => { if (primarySongTrackPath) { loadAudioVersion(primarySongTrackPath); showContextMenu = false; } }}
            title="Path: {primarySongTrackPath}"
          >
            <span class="menu-check">{filePath === primarySongTrackPath ? "✓" : " "}</span>
            <span class="menu-item-icon">🎵</span>
            <span class="menu-item-text">{primarySongTrackPath.split('/').pop() || primarySongTrackPath}</span>
            <span class="menu-item-badge badge-orig">DEFAULT</span>
          </div>
        {/if}

        <!-- Associated Audio Versions List (1-Click Switcher) -->
        {#if associatedFiles && associatedFiles.some(f => f.fileType === 'audio')}
          {#each associatedFiles.filter(f => f.fileType === 'audio') as assocItem}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div 
              class="menu-item menu-item-assoc" 
              class:menu-active={filePath === assocItem.path}
              on:click={() => { loadAudioVersion(assocItem.path); showContextMenu = false; }}
              title="Path: {assocItem.path}"
            >
              <span class="menu-check">{filePath === assocItem.path ? "✓" : " "}</span>
              <span class="menu-item-icon">
                {#if assocItem.role === 'lead' || assocItem.name.toLowerCase().includes('lead vocal')}
                  🎤
                {:else if assocItem.role === 'backings' || assocItem.name.toLowerCase().includes('backing vocal')}
                  👥
                {:else if assocItem.role === 'vocals' || assocItem.name.toLowerCase().includes('vocals ensemble') || assocItem.name.toLowerCase().includes('(vocals)')}
                  ✨
                {:else if isLosslessAudio(assocItem.path || assocItem.name)}
                  🎼
                {:else if assocItem.id === 'audio-orig' || assocItem.name.toLowerCase().includes('original')}
                  🎵
                {:else}
                  🎵
                {/if}
              </span>
              <span class="menu-item-text">{assocItem.name}</span>
              <span 
                class="menu-item-badge"
                class:badge-lossless={isLosslessAudio(assocItem.path || assocItem.name)}
                class:badge-lead={assocItem.role === 'lead' || assocItem.name.toLowerCase().includes('lead vocal')}
                class:badge-backings={assocItem.role === 'backings' || assocItem.name.toLowerCase().includes('backing vocal')}
                class:badge-vocals={assocItem.role === 'vocals' || assocItem.name.toLowerCase().includes('vocals ensemble')}
                class:badge-orig={assocItem.id === 'audio-orig' || assocItem.name.toLowerCase().includes('original')}
              >
                {#if assocItem.path === primarySongTrackPath}
                  DEFAULT
                {:else if assocItem.role === 'lead' || assocItem.name.toLowerCase().includes('lead vocal')}
                  LEAD
                {:else if assocItem.role === 'backings' || assocItem.name.toLowerCase().includes('backing vocal')}
                  BACKINGS
                {:else if assocItem.role === 'vocals' || assocItem.name.toLowerCase().includes('vocals ensemble')}
                  VOCALS
                {:else if isLosslessAudio(assocItem.path || assocItem.name)}
                  FULL-RES
                {:else if assocItem.id === 'audio-orig' || assocItem.name.toLowerCase().includes('original')}
                  ORIGINAL
                {:else}
                  AUDIO
                {/if}
              </span>
            </div>
          {/each}
        {/if}

        <div class="menu-divider"></div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item" 
          on:click={() => { triggerAbCompare(); showContextMenu = false; }}
          title="Toggle between default mix and selected stem (hotkey: T)"
        >
          <span class="menu-check">&nbsp;</span>
          <span class="menu-item-text">⇄ A/B Compare (T)</span>
        </div>

        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item" 
          class:menu-active={preferHighResAudio}
          on:click={() => { togglePreferHighResAudio(); showContextMenu = false; }}
          title="Automatically load high-res lossless audio as main track if downloaded locally"
        >
          <span class="menu-check">{preferHighResAudio ? "✓" : " "}</span>
          <span class="menu-item-text">⚡ Prefer High-Res (if downloaded)</span>
        </div>

        <div class="menu-divider"></div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div class="menu-item" on:click={() => { pickMainTrack(); showContextMenu = false; }}>
          <span class="menu-check">&nbsp;</span>
          <span class="menu-item-text">Choose / Replace Audio File...</span>
        </div>

        {#if filePath}
          <div class="menu-divider"></div>
          <div class="menu-section-header">🏷️ Audio Tags (Active Track)</div>
          {#each AUDIO_TAG_OPTIONS as tagOpt}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div 
              class="menu-item menu-item-tag"
              on:click|stopPropagation={() => toggleFileTag(filePath, tagOpt.label)}
            >
              <span class="menu-check">{getFileTags(filePath).includes(tagOpt.label) ? "✓" : " "}</span>
              <span class="tag-dot" style="background-color: {tagOpt.color};"></span>
              <span class="menu-item-text">{tagOpt.label}</span>
            </div>
          {/each}
        {/if}

      {:else if contextMenuType === "assoc-file" && contextMenuTargetAssoc}
        <!-- Associated File Card Context Menu -->
        <div class="menu-header-label" title={contextMenuTargetAssoc.name}>
          {contextMenuTargetAssoc.name}
        </div>
        <div class="menu-divider"></div>

        {#if contextMenuTargetAssoc.fileType === 'audio'}
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div 
            class="menu-item" 
            class:menu-active={filePath === contextMenuTargetAssoc.path}
            on:click={() => { if (contextMenuTargetAssoc) { loadAudioVersion(contextMenuTargetAssoc.path); showContextMenu = false; } }}
          >
            ▶ Load into Waveform
          </div>
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div 
            class="menu-item"
            on:click={() => { if (contextMenuTargetAssoc) { setDefaultSongTrack(contextMenuTargetAssoc); showContextMenu = false; } }}
          >
            ⭐️ Set as Default Song Track
          </div>

          <div class="menu-divider"></div>
          <div class="menu-section-header">🏷️ Audio Tags</div>
          {#each AUDIO_TAG_OPTIONS as tagOpt}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div 
              class="menu-item menu-item-tag"
              on:click|stopPropagation={() => { if (contextMenuTargetAssoc) toggleFileTag(contextMenuTargetAssoc.path, tagOpt.label); }}
            >
              <span class="menu-check">{getFileTags(contextMenuTargetAssoc.path, contextMenuTargetAssoc.name).includes(tagOpt.label) ? "✓" : " "}</span>
              <span class="tag-dot" style="background-color: {tagOpt.color};"></span>
              <span class="menu-item-text">{tagOpt.label}</span>
            </div>
          {/each}

          <!-- Stems options -->
          {#if !contextMenuTargetAssoc.role?.includes('lead') && !contextMenuTargetAssoc.role?.includes('backing') && !(contextMenuTargetAssoc.name && (contextMenuTargetAssoc.name.toLowerCase().includes('lead') || contextMenuTargetAssoc.name.toLowerCase().includes('backing') || contextMenuTargetAssoc.name.toLowerCase().includes('bgv')))}
            {#if contextMenuTargetAssoc.role === 'vocals' || contextMenuTargetAssoc.name.toLowerCase().includes('vocals') || contextMenuTargetAssoc.name.toLowerCase().includes('ensemble')}
              <div class="menu-divider"></div>
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div 
                class="menu-item"
                on:click={() => { if (contextMenuTargetAssoc) { const p = contextMenuTargetAssoc.path; showContextMenu = false; triggerUvrSeparation('lead_backups', p); } }}
              >
                🎤 Split Lead / Backups (5HP Karaoke)
              </div>
            {:else}
              <div class="menu-divider"></div>
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div 
                class="menu-item"
                on:click={() => { if (contextMenuTargetAssoc) { const p = contextMenuTargetAssoc.path; showContextMenu = false; triggerUvrSeparation('ensemble_vocals', p); } }}
              >
                ✨ Isolate Vocals (4-Model Ensemble)
              </div>
            {/if}
          {/if}

        {:else if contextMenuTargetAssoc.fileType === 'pdf'}
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div 
            class="menu-item" 
            on:click={() => { if (contextMenuTargetAssoc) { openPdfTab(contextMenuTargetAssoc.path, contextMenuTargetAssoc.name); showContextMenu = false; } }}
          >
            📄 Open Sheet Music in Tab
          </div>
        {/if}

        <div class="menu-divider"></div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item menu-item-danger" 
          on:click={() => { if (contextMenuTargetAssoc) { const item = contextMenuTargetAssoc; showContextMenu = false; requestMoveToTrash(item); } }}
        >
          🗑️ Move to Trash...
        </div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="menu-item" 
          on:click={() => { if (contextMenuTargetAssoc) { const id = contextMenuTargetAssoc.id; showContextMenu = false; unlinkAssociatedFile(id); } }}
        >
          × Unlink from Project
        </div>
      {:else}
        <!-- Browser / Playlist Context Menu -->
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div class="menu-item" on:click={() => { if (contextMenuTargetFile) { loadAudioVersion(contextMenuTargetFile.path); showContextMenu = false; } }}>
          ▶ Load into Waveform
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

        {#if contextMenuTargetFile && !contextMenuTargetFile.is_dir && isAudioFile(contextMenuTargetFile.name || contextMenuTargetFile.path)}
          <div class="menu-divider"></div>
          <div class="menu-section-header">🏷️ Audio Tags</div>
          {#each AUDIO_TAG_OPTIONS as tagOpt}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div 
              class="menu-item menu-item-tag"
              on:click|stopPropagation={() => { if (contextMenuTargetFile) toggleFileTag(contextMenuTargetFile.path, tagOpt.label); }}
            >
              <span class="menu-check">{getFileTags(contextMenuTargetFile.path, contextMenuTargetFile.name).includes(tagOpt.label) ? "✓" : " "}</span>
              <span class="tag-dot" style="background-color: {tagOpt.color};"></span>
              <span class="menu-item-text">{tagOpt.label}</span>
            </div>
          {/each}
        {/if}
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
            openXfadeModal(reg);
          }
        }}>
          ✕ Splice Crossfade ({contextMenuRegion.crossfadeMs ?? 5}ms)...
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
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="console-meter-col input-meter-col" on:dblclick={() => { curCompStage.thresholdDb = 0.0; updateCompressorEngine(); }} title="Double-click to reset Threshold to 0 dB">
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
                    on:dblclick|stopPropagation={() => { curCompStage.thresholdDb = 0.0; updateCompressorEngine(); }}
                    class="vert-slider thresh-slider" 
                    title="Threshold: {curCompStage.thresholdDb.toFixed(1)} dB (Double-click to reset to 0 dB)"
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
                <line x1="20" y1="180" x2="280" y2="20" stroke="rgba(255,255,255,0.2)" stroke-width="1" stroke-dasharray="3 3" />

                <!-- Dynamic Transfer Function Path -->
                <path 
                  d={getSonitusCurvePath(curCompStage)} 
                  stroke="#ffcc00" 
                  stroke-width="1" 
                  fill="none" 
                  stroke-linecap="round" 
                />

                <!-- Animated Signal Dot tracing along compression transfer line -->
                <circle 
                  cx={getSonitusSignalDot(curCompStage, liveInputPeakL).x} 
                  cy={getSonitusSignalDot(curCompStage, liveInputPeakL).y} 
                  r="4" 
                  fill="#30d158" 
                  stroke="#ffffff" 
                  stroke-width="1.5" 
                  opacity={isPlaying ? 1 : 0.4}
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

            <!-- 3. Gain Reduction Meter (Top Down) -->
            <div class="console-meter-col gr-meter-col">
              <span class="meter-label">GR</span>
              <div class="meter-slider-combo">
                <div class="gr-peak-track">
                  <div class="gr-fill" style="height: {Math.min(100, Math.max(0, liveGainReductionDb * (100 / 30)))}%;"></div>
                </div>
              </div>
              <span class="meter-val-tag gr-val">-{liveGainReductionDb.toFixed(1)} dB</span>
            </div>

            <!-- 4. Gain Makeup Vertical Slider -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="console-meter-col makeup-meter-col" on:dblclick={() => { curCompStage.makeupDb = 0.0; updateCompressorEngine(); }} title="Double-click to reset Makeup to 0 dB">
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
                    on:dblclick|stopPropagation={() => { curCompStage.makeupDb = 0.0; updateCompressorEngine(); }}
                    class="vert-slider makeup-slider" 
                    title="Makeup Gain: +{curCompStage.makeupDb.toFixed(1)} dB (Double-click to reset to 0 dB)"
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
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="param-knob-box" on:dblclick={() => { curCompStage.compType = activeCompStageTab === 1 ? 'Vintage' : 'Opto'; updateCompressorEngine(); }} title="Double-click to reset Character">
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
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="param-knob-box" on:dblclick={() => { curCompStage.ratio = 1.0; updateCompressorEngine(); }} title="Double-click to reset Ratio to 1.0:1">
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
                on:dblclick|stopPropagation={() => { curCompStage.ratio = 1.0; updateCompressorEngine(); }}
                class="rack-h-slider" 
              />
            </div>

            <!-- Knee Slider -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="param-knob-box" on:dblclick={() => { curCompStage.kneeDb = 3.0; updateCompressorEngine(); }} title="Double-click to reset Knee to 3.0 dB">
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
                on:dblclick|stopPropagation={() => { curCompStage.kneeDb = 3.0; updateCompressorEngine(); }}
                class="rack-h-slider" 
              />
            </div>

            <!-- Attack Slider -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="param-knob-box" on:dblclick={() => { curCompStage.attackMs = 30.0; updateCompressorEngine(); }} title="Double-click to reset Attack to 30.0 ms">
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
                on:dblclick|stopPropagation={() => { curCompStage.attackMs = 30.0; updateCompressorEngine(); }}
                class="rack-h-slider" 
              />
            </div>

            <!-- Release Slider -->
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="param-knob-box" on:dblclick={() => { curCompStage.releaseMs = 300.0; updateCompressorEngine(); }} title="Double-click to reset Release to 300 ms">
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
                on:dblclick|stopPropagation={() => { curCompStage.releaseMs = 300.0; updateCompressorEngine(); }}
                class="rack-h-slider" 
              />
            </div>

            {#if compRouting === 'Parallel'}
              <!-- Parallel Blend Slider -->
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div class="param-knob-box blend-box" on:dblclick={() => { compParallelBlend = 0.5; updateCompressorEngine(); }} title="Double-click to reset Blend to 50%">
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
                  on:dblclick|stopPropagation={() => { compParallelBlend = 0.5; updateCompressorEngine(); }}
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
          <!-- Precision Parametric EQ Graph Canvas -->
          <div class="helm-eq-canvas-card">
            <svg 
              bind:this={eqSvgElement}
              viewBox="0 0 600 240" 
              class="helm-eq-svg"
              on:mousedown={handleEqSvgMousedown}
              on:mousemove={handleEqSvgMousemove}
              on:mouseup={handleEqSvgMouseup}
              on:mouseleave={handleEqSvgMouseup}
              on:dblclick={handleEqSvgDblClick}
              style="cursor: {isDraggingEqNode ? 'grabbing' : 'crosshair'};"
            >
              <defs>
                <linearGradient id="rtaGrad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stop-color="#64d2ff" stop-opacity="0.35" />
                  <stop offset="50%" stop-color="#bf5af2" stop-opacity="0.15" />
                  <stop offset="100%" stop-color="#bf5af2" stop-opacity="0.0" />
                </linearGradient>
              </defs>

              <!-- Logarithmic Frequency Grid Lines (20Hz to 20kHz) -->
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

              <!-- Real-Time Spiky 64-Band FFT Spectrum Analyzer (RTA) -->
              <path 
                d={getLiveRtaSpectrumPath(isPlaying, liveInputPeakL, currentTime)} 
                fill="url(#rtaGrad)" 
                stroke="none" 
              />
              <path 
                d={getLiveRtaSpectrumPath(isPlaying, liveInputPeakL, currentTime)} 
                fill="none" 
                stroke="#64d2ff" 
                stroke-width="1" 
                opacity="0.5" 
              />

              <!-- Cumulative Exact Analytical EQ Response Curve Path -->
              <path 
                d={getEqPath(eqNodes)} 
                stroke="#64d2ff" 
                stroke-width="2" 
                fill="none" 
                stroke-linecap="round" 
              />

              <!-- Interactive Node Handles -->
              {#each eqNodes as node}
                {@const nx = 20 + ((Math.log10(Math.max(20, Math.min(20000, node.freq))) - 1.30103) / 3.0) * 560}
                {@const ny = node.enabled ? 120 - Math.max(-24, Math.min(24, node.gainDb)) * (100 / 24) : 120}
                {@const isSel = selectedEqNodeId === node.id}
                {@const qWidth = Math.max(15, 60 / node.q)}

                <!-- Selected Node Q Bandwidth Wings & Interactive Grab Handles -->
                {#if isSel}
                  <line x1={nx - qWidth} y1={ny} x2={nx + qWidth} y2={ny} stroke={node.color} stroke-width="2" stroke-dasharray="2 2" />
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <circle 
                    cx={nx - qWidth} 
                    cy={ny} 
                    r="5.5" 
                    fill={node.color} 
                    stroke="#ffffff" 
                    stroke-width="1.5" 
                    class="eq-width-handle"
                    style="cursor: ew-resize;"
                    title="Drag horizontally to adjust bandwidth / Q"
                    on:mousedown={(e) => handleEqWidthHandleMousedown(e, node.id, "left")}
                  />
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <circle 
                    cx={nx + qWidth} 
                    cy={ny} 
                    r="5.5" 
                    fill={node.color} 
                    stroke="#ffffff" 
                    stroke-width="1.5" 
                    class="eq-width-handle"
                    style="cursor: ew-resize;"
                    title="Drag horizontally to adjust bandwidth / Q"
                    on:mousedown={(e) => handleEqWidthHandleMousedown(e, node.id, "right")}
                  />
                {/if}

                <!-- Draggable Center Node Circle -->
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <circle 
                  cx={nx} 
                  cy={ny} 
                  r={isSel ? "9" : "7"} 
                  fill={node.enabled ? node.color : "#555555"} 
                  stroke="#ffffff" 
                  stroke-width={isSel ? "2.5" : "1.5"}
                  class="eq-node-handle"
                  style="cursor: grab;"
                  on:mousedown={(e) => handleEqNodeMousedown(e, node.id)}
                  on:wheel={(e) => handleEqSvgWheel(e, node)}
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
                <text x={nx} y={ny - 13} text-anchor="middle" fill="#ffffff" font-size="10" font-weight="700">
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
              <!-- Top Header Row: Node Tabs Selector (Left) & Actions/Delete/Power (Right) -->
              <div class="eq-deck-top-row">
                <div class="eq-node-pills-row">
                  {#each sortedEqNodes as node, idx}
                    <button 
                      class="eq-node-pill-btn" 
                      class:active={selectedEqNodeId === node.id}
                      class:core-node-pill={isCoreEqNode(node.id)}
                      style="--node-color: {node.color};"
                      on:click={() => selectedEqNodeId = node.id}
                      title={isCoreEqNode(node.id) ? "Primary Rack-Linked EQ Band (Permanent)" : "Custom Parametric EQ Band"}
                    >
                      <span class="pill-dot" style="background-color: {node.color};"></span>
                      <span>{idx + 1}. {node.name}</span>
                    </button>
                  {/each}
                </div>

                <!-- Top Right Action Controls for Selected Band -->
                <div class="eq-deck-top-actions">
                  <button 
                    class="eq-node-power-btn" 
                    class:active={selEqNode.enabled}
                    on:click={() => { selEqNode.enabled = !selEqNode.enabled; updateEqEngine(); }}
                    title="Toggle active / bypass state for this filter band"
                  >
                    {selEqNode.enabled ? "ACTIVE" : "MUTED"}
                  </button>

                  {#if !isCoreEqNode(selEqNode.id)}
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
                  {:else}
                    <span class="eq-core-band-badge" title="Primary rack-linked EQ bands cannot be deleted">
                      🔒 Core Band
                    </span>
                  {/if}
                </div>
              </div>

              <!-- Node Parameters Control Grid (4 Columns) -->
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

                <!-- Frequency (Direct Number Input & Logarithmic Slider) -->
                <div class="eq-ctrl-group">
                  <div class="eq-label-val-row">
                    <label class="eq-ctrl-label" for="eq-freq-input">Frequency</label>
                    <div class="eq-number-input-wrapper">
                      <input 
                        type="number" 
                        min="20" 
                        max="20000" 
                        step="1"
                        value={Math.round(selEqNode.freq)}
                        on:change={(e) => {
                          const val = parseFloat(e.currentTarget.value);
                          if (!isNaN(val)) {
                            selEqNode.freq = Math.max(20, Math.min(20000, Math.round(val)));
                            updateEqEngine();
                          }
                        }}
                        class="eq-direct-num-input"
                        title="Type exact frequency (20 - 20000 Hz)"
                      />
                      <span class="eq-unit-suffix">Hz</span>
                    </div>
                  </div>
                  <input 
                    id="eq-freq-input"
                    type="range" 
                    min="1.30103" 
                    max="4.30103" 
                    step="0.001" 
                    value={Math.log10(Math.max(20, Math.min(20000, selEqNode.freq)))} 
                    on:input={(e) => {
                      const logVal = parseFloat(e.currentTarget.value);
                      selEqNode.freq = Math.round(Math.pow(10, logVal));
                      updateEqEngine();
                    }}
                    class="rack-h-slider" 
                  />
                </div>

                <!-- Gain (for Bell and Shelves) -->
                {#if selEqNode.filterType === 'Peaking' || selEqNode.filterType === 'LowShelf' || selEqNode.filterType === 'HighShelf'}
                  <div class="eq-ctrl-group">
                    <div class="eq-label-val-row">
                      <label class="eq-ctrl-label" for="eq-gain-input">Gain</label>
                      <div class="eq-number-input-wrapper">
                        <input 
                          type="number" 
                          min="-24.0" 
                          max="24.0" 
                          step="0.1"
                          value={selEqNode.gainDb}
                          on:change={(e) => {
                            const val = parseFloat(e.currentTarget.value);
                            if (!isNaN(val)) {
                              selEqNode.gainDb = parseFloat(Math.max(-24.0, Math.min(24.0, val)).toFixed(1));
                              updateEqEngine();
                            }
                          }}
                          class="eq-direct-num-input"
                          title="Type exact gain (-24 to +24 dB)"
                        />
                        <span class="eq-unit-suffix">dB</span>
                      </div>
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
                {:else}
                  <div class="eq-ctrl-group">
                    <div class="eq-label-val-row">
                      <label class="eq-ctrl-label">Gain</label>
                      <span class="eq-unit-suffix">Fixed Cut (N/A)</span>
                    </div>
                    <div class="eq-disabled-slider-placeholder"></div>
                  </div>
                {/if}

                <!-- Q / Bandwidth -->
                <div class="eq-ctrl-group">
                  <div class="eq-label-val-row">
                    <label class="eq-ctrl-label" for="eq-q-input">Q (Bandwidth)</label>
                    <div class="eq-number-input-wrapper">
                      <input 
                        type="number" 
                        min="0.1" 
                        max="10.0" 
                        step="0.05"
                        value={selEqNode.q}
                        on:change={(e) => {
                          const val = parseFloat(e.currentTarget.value);
                          if (!isNaN(val)) {
                            selEqNode.q = parseFloat(Math.max(0.1, Math.min(10.0, val)).toFixed(2));
                            updateEqEngine();
                          }
                        }}
                        class="eq-direct-num-input"
                        title="Type exact Q bandwidth factor (0.10 - 10.00)"
                      />
                      <span class="eq-unit-suffix">Q</span>
                    </div>
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
                <input type="checkbox" bind:checked={exportBakeEnvelope} />
                <span class="check-label">
                  Volume Envelope
                  <span class="check-sub">{volumeEnvelopeNodes.length} nodes (Pre-Dynamics)</span>
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

  <!-- Cut Splice Crossfade Modal -->
  {#if showXfadeModal && editingXfadeRegion}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={closeXfadeModal}>
      <div class="inspector-modal xfade-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge cut-badge">CUT</span>
            <h3>Splice Crossfade</h3>
            <span class="stage-subhead">Region: "{editingXfadeRegion.name}"</span>
          </div>
          <button class="modal-close-btn" on:click={closeXfadeModal}>×</button>
        </div>

        <div class="modal-body xfade-modal-body">
          <div class="xfade-readout-row">
            <span class="xfade-readout-val">{editingXfadeMs}</span>
            <span class="xfade-readout-unit">ms</span>
          </div>

          <div class="xfade-slider-box">
            <div class="eq-label-val-row">
              <label class="eq-ctrl-label" for="xfade-slider">Equal-Power Crossfade Duration</label>
              <span class="eq-val-badge">{editingXfadeMs} ms</span>
            </div>
            <input 
              id="xfade-slider"
              type="range" 
              min="0" 
              max="100" 
              step="1" 
              bind:value={editingXfadeMs} 
              class="rack-h-slider"
            />
          </div>

          <div class="xfade-presets-container">
            <span class="xfade-presets-label">Quick Presets:</span>
            <div class="xfade-presets-row">
              <button class="xfade-preset-btn" class:active={editingXfadeMs === 0} on:click={() => editingXfadeMs = 0}>0ms (Hard)</button>
              <button class="xfade-preset-btn" class:active={editingXfadeMs === 5} on:click={() => editingXfadeMs = 5}>5ms (Fast)</button>
              <button class="xfade-preset-btn" class:active={editingXfadeMs === 10} on:click={() => editingXfadeMs = 10}>10ms (Standard)</button>
              <button class="xfade-preset-btn" class:active={editingXfadeMs === 25} on:click={() => editingXfadeMs = 25}>25ms (Smooth)</button>
              <button class="xfade-preset-btn" class:active={editingXfadeMs === 50} on:click={() => editingXfadeMs = 50}>50ms (Long)</button>
            </div>
          </div>

          <p class="xfade-description-hint">
            Equal-power cosine crossfading smoothly joins audio regions across the splice boundary, eliminating clicks and pop transients.
          </p>
        </div>

        <div class="modal-footer">
          <button class="modal-cancel-btn" on:click={closeXfadeModal}>Cancel</button>
          <button class="modal-action-btn" on:click={applyXfadeModal}>Save & Apply</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Application Preferences Modal (Cmd+,) -->
  {#if showPreferencesModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={closePreferencesModal}>
      <div class="inspector-modal prefs-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge prefs-badge">PREFS</span>
            <h3>Preferences</h3>
            <span class="stage-subhead">TrackHelm Application Settings</span>
          </div>
          <button class="modal-close-btn" on:click={closePreferencesModal}>×</button>
        </div>

        <div class="modal-body prefs-modal-body">
          <!-- Library & Setlist Default Folders (AI Assistant Groundwork) -->
          <div class="export-section">
            <div class="export-section-title">LIBRARY & SETLIST ASSETS FOLDERS</div>
            <p class="prefs-section-hint">
              Designate your primary rehearsal library directories. Used by the <strong>AI Setlist Importer</strong> and Media Linker to automatically discover and pair backing tracks, stems, original recordings, and sheet music.
            </p>

            <div class="prefs-folder-grid">
              <!-- 1. Low-Res Performance Tracks (AAC / M4A) -->
              <div class="prefs-folder-row">
                <div class="folder-label-cell">
                  <span class="folder-badge aac-badge">AAC</span>
                  <div class="folder-title-desc">
                    <span class="folder-title">Performance Tracks (Low-Res)</span>
                    <span class="folder-desc">Default AAC / M4A rehearsal & stage backing tracks</span>
                  </div>
                </div>
                <div class="folder-input-cell">
                  <input 
                    type="text" 
                    class="folder-path-input" 
                    placeholder="Choose folder e.g. ~/Music/Performance Tracks (AAC)..." 
                    bind:value={prefFolderAac} 
                    on:change={saveAppPreferences}
                  />
                  <button class="folder-pick-btn" on:click={() => pickPreferenceFolder('aac')}>Choose...</button>
                  {#if prefFolderAac}
                    <button class="folder-clear-btn" on:click={() => { prefFolderAac = ""; saveAppPreferences(); }} title="Clear path">×</button>
                  {/if}
                </div>
              </div>

              <!-- 2. High-Res Performance Tracks (WAV / FLAC) -->
              <div class="prefs-folder-row">
                <div class="folder-label-cell">
                  <span class="folder-badge hires-badge">WAV</span>
                  <div class="folder-title-desc">
                    <span class="folder-title">Performance Tracks (Full-Res)</span>
                    <span class="folder-desc">Master uncompressed WAV / FLAC alternate performance files</span>
                  </div>
                </div>
                <div class="folder-input-cell">
                  <input 
                    type="text" 
                    class="folder-path-input" 
                    placeholder="Choose folder e.g. ~/Music/Master Tracks (WAV)..." 
                    bind:value={prefFolderHiRes} 
                    on:change={saveAppPreferences}
                  />
                  <button class="folder-pick-btn" on:click={() => pickPreferenceFolder('hires')}>Choose...</button>
                  {#if prefFolderHiRes}
                    <button class="folder-clear-btn" on:click={() => { prefFolderHiRes = ""; saveAppPreferences(); }} title="Clear path">×</button>
                  {/if}
                </div>
              </div>

              <!-- 3. Original Artist Recordings -->
              <div class="prefs-folder-row">
                <div class="folder-label-cell">
                  <span class="folder-badge orig-badge">ORIG</span>
                  <div class="folder-title-desc">
                    <span class="folder-title">Original Artist Recordings</span>
                    <span class="folder-desc">Reference album recordings for rehearsal comparison</span>
                  </div>
                </div>
                <div class="folder-input-cell">
                  <input 
                    type="text" 
                    class="folder-path-input" 
                    placeholder="Choose folder e.g. ~/Music/Original Tracks..." 
                    bind:value={prefFolderOrig} 
                    on:change={saveAppPreferences}
                  />
                  <button class="folder-pick-btn" on:click={() => pickPreferenceFolder('orig')}>Choose...</button>
                  {#if prefFolderOrig}
                    <button class="folder-clear-btn" on:click={() => { prefFolderOrig = ""; saveAppPreferences(); }} title="Clear path">×</button>
                  {/if}
                </div>
              </div>

              <!-- 4. Sheet Music PDFs -->
              <div class="prefs-folder-row">
                <div class="folder-label-cell">
                  <span class="folder-badge pdf-badge">PDF</span>
                  <div class="folder-title-desc">
                    <span class="folder-title">Sheet Music & Chord Charts</span>
                    <span class="folder-desc">Master PDF charts, lead sheets, and vocal scores</span>
                  </div>
                </div>
                <div class="folder-input-cell">
                  <input 
                    type="text" 
                    class="folder-path-input" 
                    placeholder="Choose folder e.g. ~/Documents/Sheet Music..." 
                    bind:value={prefFolderPdf} 
                    on:change={saveAppPreferences}
                  />
                  <button class="folder-pick-btn" on:click={() => pickPreferenceFolder('pdf')}>Choose...</button>
                  {#if prefFolderPdf}
                    <button class="folder-clear-btn" on:click={() => { prefFolderPdf = ""; saveAppPreferences(); }} title="Clear path">×</button>
                  {/if}
                </div>
              </div>

              <!-- 5. Isolated Vocals & Stems (UVR) -->
              <div class="prefs-folder-row">
                <div class="folder-label-cell">
                  <span class="folder-badge vocals-badge">VOC</span>
                  <div class="folder-title-desc">
                    <span class="folder-title">Isolated Vocals & Stems (UVR)</span>
                    <span class="folder-desc">Dedicated directory for isolated vocals, lead/backing stems, and acapellas</span>
                  </div>
                </div>
                <div class="folder-input-cell">
                  <input 
                    type="text" 
                    class="folder-path-input" 
                    placeholder="Choose folder e.g. ~/Music/Vocals Only..." 
                    bind:value={prefFolderVocals} 
                    on:change={saveAppPreferences}
                  />
                  <button class="folder-pick-btn" on:click={() => pickPreferenceFolder('vocals')}>Choose...</button>
                  {#if prefFolderVocals}
                    <button class="folder-clear-btn" on:click={() => { prefFolderVocals = ""; saveAppPreferences(); }} title="Clear path">×</button>
                  {/if}
                </div>
              </div>
            </div>

            <!-- Batch Fuzzy Auto-Link Section -->
            <div class="auto-link-card">
              <div class="auto-link-info">
                <div class="auto-link-title">⚡ Batch Auto-Link Library Files</div>
                <div class="auto-link-desc">
                  Uses fuzzy matching to link performance tracks with matching originals, PDFs, and any isolated vocal/iso stems across your configured library folders.
                </div>
              </div>
              <button 
                class="auto-link-action-btn" 
                disabled={isAutoLinkingLibrary} 
                on:click={executeAutoLinkLibrary}
              >
                {#if isAutoLinkingLibrary}
                  <span class="spinner-inline"></span> Linking Library...
                {:else}
                  ⚡ Auto-Link Library Files
                {/if}
              </button>
            </div>

            {#if autoLinkStatusMessage}
              <div class="auto-link-status-banner" class:error={autoLinkStatusIsError}>
                <span>{autoLinkStatusMessage}</span>
                <button class="status-dismiss-btn" on:click={() => autoLinkStatusMessage = ""}>×</button>
              </div>
            {/if}
          </div>

          <!-- Sheet Music & PDF Theme Section -->
          <div class="export-section">
            <div class="export-section-title">SHEET MUSIC & PDF CHARTS</div>
            
            <div class="prefs-field-row">
              <span class="prefs-field-label">Default PDF Theme:</span>
              <div class="prefs-segmented-group">
                <button 
                  class="prefs-seg-btn" 
                  class:active={prefPdfDefaultTheme === 'light'} 
                  on:click={() => setPdfDefaultTheme('light')}
                >
                  <span class="seg-icon">☀️</span> Light (Normal)
                </button>
                <button 
                  class="prefs-seg-btn" 
                  class:active={prefPdfDefaultTheme === 'dark'} 
                  on:click={() => setPdfDefaultTheme('dark')}
                >
                  <span class="seg-icon">🌙</span> Dark (Inverted)
                </button>
                <button 
                  class="prefs-seg-btn" 
                  class:active={prefPdfDefaultTheme === 'system'} 
                  on:click={() => setPdfDefaultTheme('system')}
                >
                  <span class="seg-icon">💻</span> Follow OS
                </button>
              </div>
            </div>
          </div>

          <!-- AI & LLM Setlist Matcher Provider Settings -->
          <div class="export-section">
            <div class="export-section-title">AI & LLM SETLIST ASSISTANT</div>
            <p class="prefs-section-hint">
              Select your AI provider for resolving ambiguous song titles, singer assignment cues, and upgraded mix detection.
            </p>

            <div class="prefs-field-row">
              <span class="prefs-field-label">AI Engine Provider:</span>
              <select class="export-select" bind:value={prefAiProvider} on:change={saveAppPreferences}>
                <option value="builtin">⚡ Built-in Fast Fuzzy Engine (100% Free & Offline)</option>
                <option value="ollama">🦙 Local Ollama (Offline LLM on localhost:11434)</option>
                <option value="openai">✨ OpenAI (GPT-4o / GPT-4o-mini)</option>
                <option value="anthropic">🧠 Anthropic Claude (Claude 3.5 Sonnet)</option>
                <option value="gemini">♊ Google Gemini (Gemini 1.5 Flash / Pro)</option>
              </select>
            </div>

            {#if prefAiProvider === "ollama"}
              <div class="prefs-field-row" style="margin-top: 8px;">
                <span class="prefs-field-label">Ollama Host URL:</span>
                <input 
                  type="text" 
                  class="folder-path-input" 
                  placeholder="http://localhost:11434" 
                  bind:value={prefOllamaUrl} 
                  on:change={saveAppPreferences}
                />
              </div>
              <div class="prefs-field-row" style="margin-top: 8px;">
                <span class="prefs-field-label">Model Name:</span>
                <input 
                  type="text" 
                  class="folder-path-input" 
                  placeholder="llama3.2 (or mistral, qwen2.5)" 
                  bind:value={prefAiModel} 
                  on:change={saveAppPreferences}
                />
              </div>
            {:else if prefAiProvider === "openai" || prefAiProvider === "anthropic" || prefAiProvider === "gemini"}
              <div class="prefs-field-row" style="margin-top: 8px;">
                <span class="prefs-field-label">API Key:</span>
                <input 
                  type="password" 
                  class="folder-path-input" 
                  placeholder="Enter API Key (saved securely on this Mac)..." 
                  bind:value={prefAiApiKey} 
                  on:change={saveAppPreferences}
                />
              </div>
              <div class="prefs-field-row" style="margin-top: 8px;">
                <span class="prefs-field-label">Custom Model (Optional):</span>
                <input 
                  type="text" 
                  class="folder-path-input" 
                  placeholder="Default: {prefAiProvider === 'openai' ? 'gpt-4o-mini' : prefAiProvider === 'anthropic' ? 'claude-3-5-sonnet-latest' : 'gemini-1.5-flash'}" 
                  bind:value={prefAiModel} 
                  on:change={saveAppPreferences}
                />
              </div>
            {/if}
          </div>
        </div>

        <div class="modal-footer">
          <span class="footer-hint">Keyboard Shortcut: ⌘,</span>
          <button class="modal-action-btn" on:click={closePreferencesModal}>Done</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- AI Setlist CSV Importer Modal -->
  {#if showSetlistModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => showSetlistModal = false}>
      <div 
        class="inspector-modal setlist-modal-card" 
        style="width: {setlistModalWidth}px; height: {setlistModalHeight}px;"
        class:is-resizing={isResizingSetlistModal}
        on:click|stopPropagation
      >
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge ai-badge">AI SETLIST</span>
            <h3>Import Setlist from CSV / Clipboard</h3>
            <span class="stage-subhead">Automated Rehearsal Package Generator</span>
          </div>
          <div class="modal-header-actions">
            <button 
              class="modal-icon-btn" 
              on:click={toggleMaximizeSetlistModal} 
              title={isSetlistModalMaximized ? "Restore Window Size" : "Maximize Window"}
            >
              {isSetlistModalMaximized ? "❐" : "🗖"}
            </button>
            <button class="modal-close-btn" on:click={() => showSetlistModal = false}>×</button>
          </div>
        </div>

        <div class="modal-body setlist-modal-body">
          <div class="setlist-input-section">
            <div class="setlist-input-header">
              <span class="export-section-title">SETLIST CSV / TSV SOURCE</span>
              <button 
                class="mini-sample-btn load-file-btn" 
                on:click={loadSetlistCsvFile}
                title="Open a CSV, TSV, or TXT file from disk"
              >
                📂 Load CSV / TSV File...
              </button>
            </div>
            <p class="prefs-section-hint">
              Column A is the <strong>Song Title</strong>. Any additional columns (singers, parts, mics, arrangement cues) are automatically parsed into a clean Markdown table in each track's rehearsal notes.
            </p>
            <textarea 
              class="setlist-csv-textarea" 
              placeholder="Song Title, Lead Singer, Mic, Arrangement Notes&#10;Crazy Little Thing Called Love, Sarah, Mic 3, Acoustic guitar intro&#10;Superstition, Marcus, Mic 1, Horn stab cue..."
              bind:value={rawCsvText}
            ></textarea>

            <div class="setlist-action-row">
              <button class="modal-action-btn resolve-btn" disabled={!rawCsvText.trim() || isResolvingSetlist} on:click={handleResolveSetlist}>
                {#if isResolvingSetlist}
                  ⏳ Resolving Library Assets...
                {:else}
                  🪄 Match & Resolve Library Assets
                {/if}
              </button>
            </div>
          </div>

          {#if setlistResolveError}
            <div class="setlist-error-banner">⚠️ {setlistResolveError}</div>
          {/if}

          <!-- Resolved Preview Table -->
          {#if resolvedSetlist.length > 0}
            <div class="setlist-results-section">
              <div class="export-section-title">RESOLVED SETLIST PREVIEW ({resolvedSetlist.length} TRACKS)</div>
              <div class="setlist-table-scroll">
                <table class="setlist-preview-table">
                  <thead>
                    <tr>
                      <th style="width: 36px;">#</th>
                      <th>Song Title</th>
                      <th>Main Audio (AAC/M4A)</th>
                      <th>Full-Res (WAV/AIF)</th>
                      <th>Original Artist</th>
                      <th>PDF Sheet Music</th>
                      <th style="width: 90px;">Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each resolvedSetlist as item}
                      <tr class:row-unlinked={item.isPlaceholder}>
                        <td class="order-cell">{item.order}</td>
                        <td class="title-cell"><strong>{item.title}</strong></td>
                        <td class="file-match-cell" title={item.mainAacPath || "Unlinked"}>
                          {item.mainAacName || "⚠️ No file (Placeholder)"}
                        </td>
                        <td class="file-match-cell" title={item.fullResWavPath || "None"}>
                          {item.fullResWavName || "—"}
                        </td>
                        <td class="file-match-cell" title={item.originalPath || "None"}>
                          {item.originalName || "—"}
                        </td>
                        <td class="file-match-cell" title={item.pdfPath || "None"}>
                          {item.pdfName || "—"}
                        </td>
                        <td class="status-cell">
                          {#if item.confidence === "exact" || item.confidence === "high"}
                            <span class="match-badge match-ok">Matched</span>
                          {:else if item.confidence === "fuzzy"}
                            <span class="match-badge match-fuzzy">Fuzzy</span>
                          {:else}
                            <span class="match-badge match-unlinked">Unlinked</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            </div>
          {/if}
        </div>

        <div class="modal-footer setlist-modal-footer">
          <div class="footer-left-info">
            <span class="resize-hint-text">⤢ Drag corner to resize</span>
          </div>
          <div class="footer-actions-row">
            <button class="modal-cancel-btn" on:click={() => showSetlistModal = false}>Cancel</button>
            <button class="modal-action-btn" disabled={resolvedSetlist.length === 0} on:click={applyImportedSetlist}>
              🚀 Import & Load Setlist ({resolvedSetlist.length})
            </button>
          </div>
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div 
            class="modal-corner-resizer" 
            on:mousedown={startResizeSetlistModal} 
            title="Drag to resize window"
          >
            <svg width="12" height="12" viewBox="0 0 12 12">
              <line x1="10" y1="3" x2="3" y2="10" stroke="#71717a" stroke-width="1.5" stroke-linecap="round" />
              <line x1="11" y1="7" x2="7" y2="11" stroke="#71717a" stroke-width="1.5" stroke-linecap="round" />
              <line x1="11" y1="11" x2="11" y2="11" stroke="#71717a" stroke-width="1.5" stroke-linecap="round" />
            </svg>
          </div>
        </div>
      </div>
    </div>
  {/if}

  <!-- About TrackHelm Modal -->
  {#if showAboutModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={closeAboutModal}>
      <div class="inspector-modal about-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge about-badge">ABOUT</span>
            <h3>TrackHelm</h3>
            <span class="stage-subhead">Rehearsal & Performance Workstation</span>
          </div>
          <button class="modal-close-btn" on:click={closeAboutModal}>×</button>
        </div>

        <div class="modal-body about-modal-body">
          <div class="about-hero">
            <div class="about-app-title">TrackHelm</div>
            <div class="about-version-tag">Version 0.1.1</div>
            <div class="about-build-date">Build: September 2026 • 60fps Fluid Resizing Engine</div>
          </div>

          <div class="about-info-grid">
            <div class="about-info-row">
              <span class="about-info-label">Platform</span>
              <span class="about-info-val">Windows x64 / Tauri 2.0</span>
            </div>
            <div class="about-info-row">
              <span class="about-info-label">Audio DSP</span>
              <span class="about-info-val">Signalsmith Stretch • Real-time Time/Pitch Engine</span>
            </div>
            <div class="about-info-row">
              <span class="about-info-label">UI Acceleration</span>
              <span class="about-info-val">Svelte 4 + WebGL/Canvas + RAF Coalescing</span>
            </div>
          </div>
        </div>

        <div class="modal-footer about-modal-footer">
          <button class="modal-action-btn" on:click={closeAboutModal}>OK</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Playlist Health & Asset Repair Modal -->
  {#if showRepairModal}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => showRepairModal = false}>
      <div class="inspector-modal repair-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge repair-badge">REPAIR</span>
            <h3>Verify & Repair Playlist Assets</h3>
            <span class="stage-subhead">Detect Missing Files and Upgrade Mix Versions</span>
          </div>
          <button class="modal-close-btn" on:click={() => showRepairModal = false}>×</button>
        </div>

        <div class="modal-body repair-modal-body">
          <p class="prefs-section-hint">
            Scans all items in the active playlist. When older mix files are deleted or replaced by newer takes (e.g. <code>_v2.m4a</code>), TrackHelm automatically discovers the best candidate and updates your playlist while preserving all your cue markers, regions, and notes.
          </p>

          {#if isScanningHealth}
            <div class="repair-loading-state">
              <span>⏳ Scanning playlist files and library folders...</span>
            </div>
          {:else if playlistHealth.length === 0}
            <div class="repair-empty-state">Playlist is currently empty.</div>
          {:else}
            <div class="repair-items-list">
              {#each playlistHealth as h}
                <div class="repair-item-row" class:has-issue={!h.exists || h.isPlaceholder}>
                  <div class="repair-item-left">
                    <span class="repair-status-icon">
                      {#if h.exists && !h.isPlaceholder}
                        ✅
                      {:else if h.suggestedReplacement}
                        🪄
                      {:else}
                        ⚠️
                      {/if}
                    </span>
                    <div class="repair-item-details">
                      <span class="repair-track-name">{h.originalName}</span>
                      <span class="repair-track-path" title={h.originalPath}>{h.originalPath}</span>
                      {#if h.suggestedReplacement && (!h.exists || h.isPlaceholder)}
                        <div class="repair-suggestion-box">
                          <span class="suggest-label">Suggested Replacement / New Mix:</span>
                          <span class="suggest-name">{h.suggestedReplacement.name}</span>
                          <span class="suggest-conf">({h.replacementConfidence} confidence)</span>
                        </div>
                      {/if}
                    </div>
                  </div>
                  <div class="repair-item-badge-wrap">
                    {#if h.exists && !h.isPlaceholder}
                      <span class="health-badge health-ok">Active</span>
                    {:else if h.suggestedReplacement}
                      <span class="health-badge health-upgrade">Upgrade Ready</span>
                    {:else}
                      <span class="health-badge health-missing">Missing Audio</span>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>

            <!-- Selective Migration Checkboxes -->
            <div class="repair-migration-section">
              <div class="repair-migration-title">METADATA MIGRATION OPTIONS FOR UPGRADED MIXES</div>
              <div class="repair-migration-grid">
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateMarkers} />
                  <span>📍 Cue Markers & Landmarks</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateRegions} />
                  <span>🔁 Loop & Cut Regions</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateNotes} />
                  <span>📝 Singer Assignments & Notes</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateVolume} />
                  <span>🔊 Base Track Volume</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateEnvelope} />
                  <span>📈 Volume Envelope Automation</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateEq} />
                  <span>🎛️ Parametric EQ Settings</span>
                </label>
                <label class="repair-check-label">
                  <input type="checkbox" bind:checked={repairMigrateComp} />
                  <span>🗜️ Compressor Dynamics</span>
                </label>
              </div>
            </div>
          {/if}
        </div>

        <div class="modal-footer">
          <button class="modal-cancel-btn" on:click={() => showRepairModal = false}>Close</button>
          <button 
            class="modal-action-btn" 
            disabled={!playlistHealth.some(h => h.suggestedReplacement && (!h.exists || h.isPlaceholder))} 
            on:click={applyRepairs}
          >
            🪄 Apply All Repairs
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if showTrashModal && itemToTrash}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div class="modal-backdrop" on:click={() => !trashLoading && (showTrashModal = false)}>
      <div class="inspector-modal trash-modal-card" on:click|stopPropagation>
        <div class="modal-header">
          <div class="modal-title-row">
            <span class="modal-badge trash-badge">TRASH</span>
            <h3>Move File to macOS Trash?</h3>
          </div>
          <button class="modal-close-btn" disabled={trashLoading} on:click={() => showTrashModal = false}>×</button>
        </div>

        <div class="modal-body trash-modal-body">
          <p class="trash-question">
            Are you sure you want to move this file to your computer's Trash?
          </p>

          <div class="trash-file-details">
            <div class="trash-file-name-row">
              <span class="trash-file-icon">{itemToTrash.fileType === 'pdf' ? '📄' : '🎵'}</span>
              <span class="trash-file-name">{itemToTrash.name}</span>
              <span class="assoc-ext-badge">{getFileExtension(itemToTrash.path || itemToTrash.name)}</span>
            </div>
            <div class="trash-file-path-row" title={itemToTrash.path}>
              <span class="trash-path-label">Path:</span>
              <span class="trash-path-value">{itemToTrash.path}</span>
            </div>
          </div>

          <div class="trash-safety-note">
            <span class="trash-safety-icon">🛡️</span>
            <span>This file will be safely moved to your macOS <strong>Trash</strong> (<code>~/.Trash</code>) and unlinked from TrackHelm. It will not be permanently deleted, and you can recover it from Finder if needed.</span>
          </div>

          {#if trashErrorMessage}
            <div class="trash-error-alert">
              <span>⚠️ {trashErrorMessage}</span>
            </div>
          {/if}
        </div>

        <div class="modal-footer">
          <button class="modal-cancel-btn" disabled={trashLoading} on:click={() => showTrashModal = false}>Cancel</button>
          <button 
            class="modal-action-btn trash-danger-btn" 
            disabled={trashLoading} 
            on:click={executeMoveToTrash}
          >
            {#if trashLoading}
              ⏳ Moving to Trash...
            {:else}
              🗑️ Move to Trash
            {/if}
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
    width: 100%;
    height: 100vh;
    box-sizing: border-box;
    background-color: #181818;
  }

  /* Grid layout spanning 3-column workspaces (Full 100vh Height) */
  .workspace-grid {
    display: grid;
    width: 100%;
    flex-grow: 1;
    overflow: hidden;
    height: 100vh;
  }

  .workspace-grid.resizing-active {
    user-select: none !important;
    cursor: col-resize !important;
  }

  .resizer-handle {
    width: 4px;
    background-color: #2b2b30;
    cursor: col-resize;
    position: relative;
    z-index: 25;
    transition: background-color 0.15s ease;
    user-select: none;
  }

  .resizer-handle:hover,
  .resizer-handle.active {
    background-color: #007aff;
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
    position: relative;
    background-color: #252526;
    border-right: 1px solid #3c3c3c;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .typeahead-indicator {
    position: absolute;
    bottom: 12px;
    right: 12px;
    background: rgba(26, 26, 32, 0.94);
    border: 1px solid #3b99fc;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.65), 0 0 8px rgba(59, 153, 252, 0.25);
    border-radius: 6px;
    padding: 5px 11px;
    font-size: 0.74rem;
    color: #ffffff;
    display: flex;
    align-items: center;
    gap: 6px;
    pointer-events: none;
    z-index: 100;
    backdrop-filter: blur(8px);
    animation: typeaheadFadeIn 0.15s ease-out;
  }

  .typeahead-indicator .typeahead-icon {
    font-size: 0.85rem;
  }

  .typeahead-indicator .typeahead-text {
    font-size: 0.72rem;
    color: #d1d1d6;
  }

  .typeahead-indicator strong {
    color: #3b99fc;
    font-weight: 700;
    letter-spacing: 0.5px;
  }

  @keyframes typeaheadFadeIn {
    from { opacity: 0; transform: translateY(4px) scale(0.96); }
    to { opacity: 1; transform: translateY(0) scale(1); }
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
    transition: color 0.15s ease, border-color 0.15s ease, background-color 0.15s ease;
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

  .browser-quick-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    background-color: #171719;
    border-bottom: 1px solid #2d2d34;
  }

  .quick-jump-btn {
    background: #242429;
    border: 1px solid #3d3d46;
    color: #a1a1aa;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    transition: background-color 0.12s, color 0.12s, border-color 0.12s;
    user-select: none;
    white-space: nowrap;
  }

  .quick-jump-btn:hover {
    background: #33333b;
    color: #ffffff;
    border-color: #52525b;
  }

  .quick-jump-btn.active {
    background: #133a60;
    color: #93c5fd;
    border-color: #3b82f6;
    font-weight: 700;
  }

  .cloud-btn-wrapper {
    position: relative;
    display: inline-block;
  }

  .cloud-jump-btn {
    background: #192533;
    border-color: #2b3e52;
    color: #7dd3fc;
  }

  .cloud-jump-btn:hover {
    background: #22374d;
    border-color: #38bdf8;
    color: #ffffff;
  }

  .cloud-jump-btn.active {
    background: #0369a1;
    color: #ffffff;
    border-color: #38bdf8;
  }

  .cloud-dropdown-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 1000;
    background: #1c1d22;
    border: 1px solid #3e3f49;
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.65);
    min-width: 220px;
    max-width: 320px;
    padding: 4px 0;
    display: flex;
    flex-direction: column;
  }

  .cloud-dropdown-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: transparent;
    border: none;
    color: #e4e4e7;
    font-size: 0.72rem;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
    width: 100%;
    box-sizing: border-box;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cloud-dropdown-item:hover {
    background: #272730;
    color: #ffffff;
  }

  .cloud-dropdown-item.active {
    background: #1e3a5f;
    color: #60a5fa;
    font-weight: 600;
  }

  .cloud-item-icon {
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .cloud-item-name {
    overflow: hidden;
    text-overflow: ellipsis;
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

  .browser-item-tags {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    margin-left: auto;
  }

  .browser-mini-tag {
    font-size: 0.58rem;
    font-weight: 700;
    padding: 0 4px;
    height: 14px;
    line-height: 14px;
    border-radius: 3px;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    pointer-events: none;
  }

  .browser-mini-tag.mini-orig {
    background-color: rgba(208, 132, 255, 0.2);
    color: #d084ff;
    border: 1px solid rgba(208, 132, 255, 0.45);
  }

  .browser-mini-tag.mini-vocals {
    background-color: rgba(48, 209, 88, 0.2);
    color: #30d158;
    border: 1px solid rgba(48, 209, 88, 0.45);
  }

  .browser-mini-tag.mini-lead {
    background-color: rgba(255, 214, 10, 0.2);
    color: #ffd60a;
    border: 1px solid rgba(255, 214, 10, 0.45);
  }

  .browser-mini-tag.mini-backings {
    background-color: rgba(100, 210, 255, 0.2);
    color: #64d2ff;
    border: 1px solid rgba(100, 210, 255, 0.45);
  }

  .browser-mini-tag.mini-iso {
    background-color: rgba(56, 189, 248, 0.2);
    color: #38bdf8;
    border: 1px solid rgba(56, 189, 248, 0.45);
  }

  .browser-mini-tag.mini-track {
    background-color: rgba(255, 159, 10, 0.2);
    color: #ff9f0a;
    border: 1px solid rgba(255, 159, 10, 0.45);
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

  .playlist-item-sidebar.dragging {
    opacity: 0.4;
    background-color: #17181c;
  }

  .playlist-item-sidebar.drag-over {
    border-top: 2px solid #3b99fc;
    background-color: rgba(59, 153, 252, 0.15);
  }

  .playlist-drag-handle {
    color: #555760;
    font-size: 0.75rem;
    cursor: grab;
    user-select: none;
    letter-spacing: -2px;
    margin-right: 2px;
    opacity: 0.6;
    transition: opacity 0.12s ease;
  }

  .playlist-item-sidebar:hover .playlist-drag-handle {
    opacity: 1;
    color: #8e94a5;
  }

  .playlist-item-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
    opacity: 0.4;
    transition: opacity 0.15s ease;
  }

  .playlist-item-sidebar:hover .playlist-item-actions,
  .playlist-item-sidebar.highlighted .playlist-item-actions {
    opacity: 1;
  }

  .reorder-item-btn {
    background: transparent;
    border: none;
    color: #8e94a5;
    cursor: pointer;
    font-size: 0.58rem;
    padding: 2px 4px;
    border-radius: 2px;
    line-height: 1;
    transition: all 0.12s ease;
  }

  .reorder-item-btn:hover:not(:disabled) {
    background-color: #383b47;
    color: #ffffff;
  }

  .reorder-item-btn:disabled {
    opacity: 0.2;
    cursor: default;
  }

  .remove-playlist-item-btn {
    background: transparent;
    border: none;
    color: #717171;
    cursor: pointer;
    font-size: 0.9rem;
    font-weight: bold;
    padding: 0 4px;
    line-height: 1;
  }

  .remove-playlist-item-btn:hover {
    color: #ff453a;
  }

  .playlist-controls-sidebar {
    background-color: #202021;
    border-top: 1px solid #3c3c3c;
    padding: 8px 10px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .playlist-controls-sidebar .action-btn {
    flex: 1 1 calc(33.333% - 6px);
    min-width: 60px;
    padding: 5px 6px;
    font-size: 0.7rem;
    text-align: center;
    justify-content: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
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

  .waveform-resize-handle {
    height: 8px;
    margin: 1px 0;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: row-resize;
    user-select: none;
    position: relative;
    z-index: 10;
    opacity: 0.55;
    transition: opacity 0.15s ease;
  }

  .waveform-resize-handle:hover,
  .waveform-resize-handle.is-resizing {
    opacity: 1;
  }

  .resize-handle-line {
    flex: 1;
    height: 1px;
    background: linear-gradient(90deg, rgba(255, 255, 255, 0.04) 0%, rgba(255, 255, 255, 0.22) 50%, rgba(255, 255, 255, 0.04) 100%);
  }

  .resize-handle-grip {
    width: 36px;
    height: 4px;
    background-color: #555560;
    border-radius: 2px;
    margin: 0 8px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.6);
  }

  .waveform-resize-handle:hover .resize-handle-grip,
  .waveform-resize-handle.is-resizing .resize-handle-grip {
    background-color: #3b99fc;
    box-shadow: 0 0 6px rgba(59, 153, 252, 0.7);
  }

  .block-main-waveform {
    flex-shrink: 0;
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

  .ab-compare-btn {
    background-color: #2a2d34;
    color: #9ab4d0;
    border-color: #3b4657;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 0.78rem;
  }

  .ab-compare-btn:hover:not(.disabled-btn) {
    background-color: #384050;
    color: #c0d8f8;
    border-color: #4f6382;
  }

  .ab-compare-btn.ab-active {
    background: linear-gradient(135deg, #104432, #186348);
    color: #64e3a5;
    border-color: #28a76f;
    box-shadow: 0 0 8px rgba(40, 167, 111, 0.4);
    font-weight: 700;
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

  .menu-section-header {
    font-size: 0.64rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #8e8e93;
    padding: 6px 14px 2px 14px;
    font-weight: 700;
  }

  .menu-header-label {
    font-size: 0.74rem;
    font-weight: 700;
    color: #ffffff;
    padding: 4px 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 280px;
  }

  .menu-item-assoc {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 360px;
  }

  .menu-item-tag {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 14px;
    cursor: pointer;
    font-size: 0.78rem;
    user-select: none;
    transition: background 0.12s ease;
  }

  .menu-item-tag:hover {
    background-color: #2c2c32;
  }

  .tag-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .menu-check {
    width: 14px;
    display: inline-block;
    color: #3b99fc;
    font-weight: 700;
    flex-shrink: 0;
    text-align: center;
  }

  .menu-item-icon {
    flex-shrink: 0;
    font-size: 0.85rem;
  }

  .menu-item-text {
    flex-grow: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .menu-item-badge {
    font-size: 0.55rem;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 3px;
    letter-spacing: 0.04em;
    flex-shrink: 0;
    background: rgba(255, 255, 255, 0.1);
    color: #d1d1d1;
  }

  .menu-item-badge.badge-lossless {
    background-color: rgba(59, 153, 252, 0.2);
    color: #3b99fc;
  }

  .menu-item-badge.badge-lead {
    background-color: rgba(255, 214, 10, 0.2);
    color: #ffd60a;
  }

  .menu-item-badge.badge-backings {
    background-color: rgba(100, 210, 255, 0.2);
    color: #64d2ff;
  }

  .menu-item-badge.badge-vocals {
    background-color: rgba(175, 82, 222, 0.2);
    color: #bf5af2;
  }

  .menu-item-badge.badge-orig {
    background-color: rgba(255, 149, 0, 0.2);
    color: #ff9f0a;
  }

  .menu-item-badge.badge-track {
    background-color: rgba(255, 159, 10, 0.2);
    color: #ff9f0a;
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
    transition: background-color 0.15s ease, filter 0.15s ease;
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
    transition: background-color 0.12s ease, border-color 0.12s ease, opacity 0.12s ease;
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
    border-bottom: 1px solid #28282e;
    padding-bottom: 6px;
    margin-bottom: 8px;
    gap: 12px;
  }

  .files-header-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .files-pane-title {
    font-size: 0.78rem;
    font-weight: 800;
    letter-spacing: 0.05em;
    color: #ffffff;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .files-count-pill {
    font-size: 0.65rem;
    font-weight: 700;
    background-color: #2c2c34;
    color: #5ac8fa;
    padding: 1px 6px;
    border-radius: 10px;
    border: 1px solid #3e3e4a;
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

  .assoc-files-table {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
  }

  .assoc-file-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 10px;
    background-color: #17171a;
    border: 1px solid #28282e;
    border-radius: 4px;
    transition: all 0.12s ease;
    user-select: none;
    cursor: pointer;
  }

  .assoc-file-row:hover {
    background-color: #202026;
    border-color: #3e3e4a;
  }

  .assoc-file-row.is-active-waveform {
    background-color: rgba(59, 153, 252, 0.08);
    border-color: rgba(59, 153, 252, 0.45);
    box-shadow: 0 0 8px rgba(59, 153, 252, 0.15);
  }

  .assoc-row-left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }

  .assoc-row-icon {
    font-size: 0.95rem;
    flex-shrink: 0;
    width: 20px;
    text-align: center;
  }

  .assoc-row-name {
    font-size: 0.78rem;
    font-weight: 600;
    color: #f2f2f7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 360px;
  }

  .assoc-row-badges {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .assoc-row-actions {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }

  .assoc-ext-badge {
    font-size: 0.58rem;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 3px;
    letter-spacing: 0.05em;
    flex-shrink: 0;
    text-transform: uppercase;
  }

  .assoc-ext-badge.badge-lossless {
    background-color: rgba(59, 153, 252, 0.18);
    color: #3b99fc;
    border: 1px solid rgba(59, 153, 252, 0.4);
  }

  .assoc-ext-badge.badge-lossy {
    background-color: rgba(48, 209, 88, 0.18);
    color: #30d158;
    border: 1px solid rgba(48, 209, 88, 0.4);
  }

  .assoc-ext-badge.badge-pdf {
    background-color: rgba(255, 149, 0, 0.18);
    color: #ff9500;
    border: 1px solid rgba(255, 149, 0, 0.4);
  }

  .assoc-role-badge {
    font-size: 0.58rem;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
    letter-spacing: 0.05em;
    flex-shrink: 0;
  }

  .assoc-role-badge.badge-orig {
    background-color: rgba(191, 90, 242, 0.22);
    color: #d084ff;
    border: 1px solid rgba(191, 90, 242, 0.5);
  }

  .assoc-role-badge.badge-hires {
    background-color: rgba(59, 153, 252, 0.22);
    color: #5ac8fa;
    border: 1px solid rgba(59, 153, 252, 0.5);
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

  .assoc-card.vocal-card {
    border-left: 3px solid #bf5af2 !important;
  }

  .assoc-card.lead-card {
    border-left: 3px solid #ffd60a !important;
  }

  .assoc-card.backing-card {
    border-left: 3px solid #64d2ff !important;
  }

  .assoc-role-badge.badge-lead {
    background-color: rgba(255, 214, 10, 0.2);
    color: #ffd60a;
    border: 1px solid rgba(255, 214, 10, 0.5);
  }

  .assoc-role-badge.badge-backings {
    background-color: rgba(100, 210, 255, 0.2);
    color: #64d2ff;
    border: 1px solid rgba(100, 210, 255, 0.5);
  }

  .assoc-role-badge.badge-track {
    background-color: rgba(255, 159, 10, 0.22);
    color: #ff9f0a;
    border: 1px solid rgba(255, 159, 10, 0.55);
    font-weight: 700;
  }

  .assoc-role-badge.badge-iso-track {
    background-color: rgba(56, 189, 248, 0.22);
    color: #38bdf8;
    border: 1px solid rgba(56, 189, 248, 0.55);
    font-weight: 700;
  }

  .assoc-role-badge.badge-vocals {
    background-color: rgba(191, 90, 242, 0.25);
    color: #d084ff;
    border: 1px solid rgba(191, 90, 242, 0.55);
  }

  .assoc-role-badge.badge-active-waveform {
    background-color: rgba(59, 153, 252, 0.25);
    color: #5ac8fa;
    border: 1px solid #3b99fc;
    font-weight: 800;
  }

  .assoc-role-badge.badge-default-mix {
    background-color: rgba(52, 199, 89, 0.2);
    color: #55e084;
    border: 1px solid rgba(52, 199, 89, 0.55);
    font-weight: 700;
  }

  .files-header-right {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .prefer-hires-btn {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.15);
    color: #a1a1aa;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 5px 11px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .prefer-hires-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #ffffff;
    border-color: rgba(255, 255, 255, 0.3);
  }

  .prefer-hires-btn.active {
    background: linear-gradient(135deg, rgba(59, 153, 252, 0.28), rgba(175, 82, 222, 0.28));
    border-color: #3b99fc;
    color: #5ac8fa;
    box-shadow: 0 0 10px rgba(59, 153, 252, 0.35);
  }

  .assoc-action-btn.uvr-action-btn {
    background-color: rgba(191, 90, 242, 0.14);
    border-color: rgba(191, 90, 242, 0.35);
    color: #d084ff;
  }

  .assoc-action-btn.uvr-action-btn:hover:not(:disabled) {
    background-color: rgba(191, 90, 242, 0.3);
    color: #ffffff;
    border-color: #d084ff;
  }

  .assoc-action-btn.uvr-karaoke-btn {
    background-color: rgba(255, 214, 10, 0.14);
    border-color: rgba(255, 214, 10, 0.35);
    color: #ffd60a;
  }

  .assoc-action-btn.uvr-karaoke-btn:hover:not(:disabled) {
    background-color: rgba(255, 214, 10, 0.3);
    color: #ffffff;
    border-color: #ffd60a;
  }

  .assoc-action-btn.trash-action-btn {
    color: #8e8e96;
    padding: 2px 6px;
  }

  .assoc-action-btn.trash-action-btn:hover {
    color: #ff453a;
    background-color: rgba(255, 69, 58, 0.18);
    border-color: rgba(255, 69, 58, 0.4);
  }

  .uvr-tab-indicator {
    font-size: 0.65rem;
    font-weight: 700;
    color: #d084ff;
    background: rgba(191, 90, 242, 0.18);
    padding: 1px 6px;
    border-radius: 10px;
    margin-left: 6px;
  }

  .files-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .uvr-header-btn {
    background-color: #7b2cbf !important;
    border: 1px solid #9d4edd !important;
  }

  .uvr-header-btn:hover:not(:disabled) {
    background-color: #9d4edd !important;
  }

  .uvr-status-bar {
    background: linear-gradient(90deg, #1e1528, #18181f);
    border: 1px solid rgba(191, 90, 242, 0.4);
    border-radius: 4px;
    padding: 4px 10px;
    margin-bottom: 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.74rem;
  }

  .uvr-sparkle {
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .uvr-stage {
    font-weight: 700;
    color: #ffffff;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .uvr-target {
    color: #8e8e93;
    font-size: 0.68rem;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex-shrink: 1;
  }

  .uvr-meter-track {
    flex: 1;
    min-width: 60px;
    height: 4px;
    background-color: #2a2035;
    border-radius: 2px;
    overflow: hidden;
  }

  .uvr-meter-fill {
    height: 100%;
    background: linear-gradient(90deg, #9d4edd, #007aff);
    border-radius: 2px;
    transition: width 0.25s ease;
  }

  .uvr-pct {
    font-size: 0.74rem;
    font-weight: 800;
    color: #d084ff;
    flex-shrink: 0;
    margin-left: auto;
  }

  .uvr-toast {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 600;
    margin-bottom: 12px;
  }

  .uvr-toast-error {
    background-color: rgba(255, 69, 58, 0.15);
    border: 1px solid rgba(255, 69, 58, 0.4);
    color: #ff6961;
  }

  .uvr-toast-success {
    background-color: rgba(48, 209, 88, 0.15);
    border: 1px solid rgba(48, 209, 88, 0.4);
    color: #30d158;
  }

  .uvr-dismiss-btn {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 1rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  /* Trash Modal */
  .trash-modal-card {
    width: 480px;
    max-width: 92vw;
  }

  .trash-badge {
    background-color: #ff453a;
  }

  .trash-modal-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .trash-question {
    margin: 0;
    font-size: 0.88rem;
    color: #ffffff;
    font-weight: 600;
  }

  .trash-file-details {
    background-color: #141416;
    border: 1px solid #28282e;
    border-radius: 6px;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .trash-file-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.85rem;
    font-weight: 700;
    color: #ffffff;
  }

  .trash-file-path-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: 0.68rem;
    color: #8e8e93;
    font-family: monospace;
    word-break: break-all;
  }

  .trash-path-label {
    color: #636366;
    flex-shrink: 0;
  }

  .trash-safety-note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    background-color: rgba(0, 122, 255, 0.1);
    border: 1px solid rgba(0, 122, 255, 0.25);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 0.72rem;
    color: #9ac9ff;
    line-height: 1.4;
  }

  .trash-safety-icon {
    font-size: 1rem;
    flex-shrink: 0;
  }

  .trash-error-alert {
    background-color: rgba(255, 69, 58, 0.15);
    border: 1px solid rgba(255, 69, 58, 0.4);
    color: #ff6961;
    padding: 8px 10px;
    border-radius: 6px;
    font-size: 0.75rem;
  }

  .trash-danger-btn {
    background-color: #ff453a !important;
    color: #ffffff !important;
  }

  .trash-danger-btn:hover:not(:disabled) {
    background-color: #ff5b52 !important;
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
    transition: background-color 0.12s ease, color 0.12s ease, filter 0.12s ease;
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
    transition: background-color 0.12s ease, color 0.12s ease;
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
    justify-content: flex-start;
    overflow: hidden;
    padding: 1px;
  }

  .gr-fill {
    width: 100%;
    background: linear-gradient(to bottom, #ff9f0a, #ff453a);
    transition: height 0.05s ease-out;
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

  /* Precision Parametric EQ Canvas & Node Controls */
  .helm-eq-canvas-card {
    position: relative;
    background-color: #08080a;
    border: 1px solid #222228;
    border-radius: 6px;
    overflow: hidden;
  }

  .helm-eq-svg {
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

  .eq-width-handle {
    cursor: ew-resize;
    transition: r 0.12s ease, stroke-width 0.12s ease;
  }

  .eq-width-handle:hover {
    r: 7;
    stroke-width: 2.5;
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

  .eq-node-pill-btn.core-node-pill {
    border: 1.5px solid #8e8e93;
    background-color: #232328;
    color: #e4e4e7;
  }

  .eq-node-pill-btn.core-node-pill:hover {
    border-color: #d1d1d6;
    background-color: #2e2e36;
    color: #ffffff;
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
    box-shadow: 0 0 0 1px var(--node-color);
  }

  .eq-node-pill-btn.core-node-pill.active {
    border-color: var(--node-color);
    box-shadow: 0 0 0 1.5px #8e8e93, 0 0 8px rgba(255, 255, 255, 0.15);
  }

  .pill-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .eq-deck-top-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 2px;
  }

  .eq-node-pills-row {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    flex: 1;
    align-items: center;
  }

  .eq-deck-top-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .eq-node-controls-grid {
    display: grid;
    grid-template-columns: 190px 1fr 1fr 1fr;
    gap: 14px;
    align-items: center;
  }

  .eq-disabled-slider-placeholder {
    height: 16px;
    opacity: 0.2;
    border-bottom: 1px dashed #555562;
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

  .eq-number-input-wrapper {
    display: inline-flex;
    align-items: center;
    background-color: #161619;
    border: 1px solid #383842;
    border-radius: 4px;
    padding: 1px 5px;
    gap: 3px;
    transition: border-color 0.12s ease, box-shadow 0.12s ease;
  }

  .eq-number-input-wrapper:focus-within {
    border-color: #64d2ff;
    box-shadow: 0 0 0 1px rgba(100, 210, 255, 0.35);
  }

  .eq-direct-num-input {
    background: transparent;
    border: none;
    outline: none;
    color: #64d2ff;
    font-size: 0.68rem;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-weight: 700;
    width: 48px;
    text-align: right;
    padding: 0;
    -moz-appearance: textfield;
  }

  .eq-direct-num-input::-webkit-outer-spin-button,
  .eq-direct-num-input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .eq-unit-suffix {
    font-size: 0.62rem;
    font-weight: 600;
    color: #8e8e96;
  }

  .actions-group {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }

  .eq-core-band-badge {
    font-size: 0.65rem;
    font-weight: 600;
    color: #8e8e96;
    padding: 5px 8px;
    background-color: rgba(255, 255, 255, 0.04);
    border: 1px solid #383842;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
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

  /* Crossfade & Preferences Modal Styles */
  .xfade-modal-card {
    width: 440px;
    max-width: 95vw;
  }

  .prefs-modal-card {
    width: 520px;
    max-width: 95vw;
  }

  .cut-badge {
    background: #ff453a;
    color: #ffffff;
  }

  .prefs-badge {
    background: #bf5af2;
    color: #ffffff;
  }

  .xfade-modal-body, .prefs-modal-body {
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .xfade-readout-row {
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: 6px;
    padding: 10px 0 6px;
  }

  .xfade-readout-val {
    font-size: 2.8rem;
    font-weight: 800;
    color: #ff453a;
    font-family: -apple-system, BlinkMacSystemFont, monospace;
  }

  .xfade-readout-unit {
    font-size: 1rem;
    font-weight: 600;
    color: #a1a1aa;
  }

  .xfade-slider-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .xfade-presets-container {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .xfade-presets-label {
    font-size: 0.7rem;
    font-weight: 700;
    color: #8e8e93;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .xfade-presets-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .xfade-preset-btn {
    background: #27272e;
    border: 1px solid #383840;
    color: #d1d1d6;
    padding: 5px 10px;
    border-radius: 6px;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .xfade-preset-btn:hover {
    background: #383842;
    color: #ffffff;
    border-color: #555562;
  }

  .xfade-preset-btn.active {
    background: #ff453a;
    color: #ffffff;
    border-color: #ff453a;
    box-shadow: 0 2px 8px rgba(255, 69, 58, 0.4);
  }

  .xfade-description-hint {
    font-size: 0.7rem;
    color: #8e8e93;
    line-height: 1.4;
    margin: 0;
    padding: 8px 10px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 6px;
    border-left: 3px solid #ff453a;
  }

  .modal-cancel-btn {
    background: transparent;
    border: 1px solid #383840;
    color: #a1a1aa;
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 0.78rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .modal-cancel-btn:hover {
    background: #27272e;
    color: #ffffff;
  }

  .prefs-field-row {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 10px;
  }

  .prefs-field-label {
    font-size: 0.75rem;
    font-weight: 700;
    color: #e4e4e7;
  }

  .prefs-segmented-group {
    display: flex;
    background: #18181c;
    border: 1px solid #323238;
    border-radius: 8px;
    padding: 3px;
    gap: 3px;
  }

  .prefs-seg-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: transparent;
    border: none;
    color: #a1a1aa;
    font-size: 0.74rem;
    font-weight: 600;
    padding: 8px 12px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .prefs-seg-btn:hover {
    color: #ffffff;
    background: rgba(255, 255, 255, 0.05);
  }

  .prefs-seg-btn.active {
    background: #3b99fc;
    color: #ffffff;
    box-shadow: 0 2px 8px rgba(59, 153, 252, 0.35);
  }

  .prefs-theme-explanation {
    font-size: 0.72rem;
    color: #8e8e93;
    line-height: 1.4;
    margin: 12px 0 0 0;
    padding: 8px 12px;
    background: rgba(59, 153, 252, 0.06);
    border-left: 3px solid #3b99fc;
    border-radius: 4px;
  }

  .prefs-section-hint {
    font-size: 0.72rem;
    color: #8e8e93;
    line-height: 1.4;
    margin: 4px 0 12px 0;
  }

  .prefs-folder-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .prefs-folder-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: #141417;
    border: 1px solid #26262c;
    border-radius: 6px;
    padding: 8px 10px;
  }

  .folder-label-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .folder-badge {
    font-size: 0.62rem;
    font-weight: 800;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: 3px;
    color: #ffffff;
  }

  .aac-badge {
    background-color: #3b99fc;
  }

  .hires-badge {
    background-color: #30d158;
  }

  .orig-badge {
    background-color: #ff9f0a;
  }

  .pdf-badge {
    background-color: #ff375f;
  }

  .vocals-badge {
    background-color: #af52de;
  }

  .folder-title-desc {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .folder-title {
    font-size: 0.75rem;
    font-weight: 700;
    color: #f2f2f7;
  }

  .folder-desc {
    font-size: 0.65rem;
    color: #8e8e93;
  }

  .folder-input-cell {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .folder-path-input {
    flex: 1;
    background: #1c1c20;
    border: 1px solid #333338;
    color: #64d2ff;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-size: 0.72rem;
    padding: 5px 8px;
    border-radius: 4px;
    outline: none;
    transition: border-color 0.12s ease;
  }

  .folder-path-input:focus {
    border-color: #3b99fc;
    box-shadow: 0 0 0 1px rgba(59, 153, 252, 0.3);
  }

  .folder-pick-btn {
    background: #282830;
    border: 1px solid #3a3a44;
    color: #e4e4e7;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 5px 10px;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.12s ease;
  }

  .folder-pick-btn:hover {
    background: #383842;
    color: #ffffff;
  }

  .folder-clear-btn {
    background: transparent;
    border: 1px solid #3a3a44;
    color: #ff453a;
    font-size: 0.8rem;
    padding: 3px 7px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .folder-clear-btn:hover {
    background: rgba(255, 69, 58, 0.15);
    border-color: #ff453a;
  }

  /* Batch Auto-Link Card */
  .auto-link-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    background: #1e1e24;
    border: 1px solid #2e2e38;
    border-radius: 6px;
    padding: 12px 14px;
    margin-top: 14px;
  }

  .auto-link-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .auto-link-title {
    font-size: 0.8rem;
    font-weight: 700;
    color: #f2f2f7;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .auto-link-desc {
    font-size: 0.68rem;
    color: #98989f;
    line-height: 1.35;
    max-width: 520px;
  }

  .auto-link-action-btn {
    background: linear-gradient(135deg, #2563eb, #1d4ed8);
    border: 1px solid #3b82f6;
    color: #ffffff;
    font-size: 0.75rem;
    font-weight: 700;
    padding: 7px 14px;
    border-radius: 5px;
    cursor: pointer;
    white-space: nowrap;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: all 0.15s ease;
    box-shadow: 0 2px 6px rgba(37, 99, 235, 0.25);
  }

  .auto-link-action-btn:hover:not(:disabled) {
    background: linear-gradient(135deg, #1d4ed8, #1e40af);
    border-color: #60a5fa;
    transform: translateY(-1px);
    box-shadow: 0 4px 10px rgba(37, 99, 235, 0.35);
  }

  .auto-link-action-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .auto-link-status-banner {
    margin-top: 10px;
    padding: 8px 12px;
    border-radius: 5px;
    background: rgba(48, 209, 88, 0.12);
    border: 1px solid rgba(48, 209, 88, 0.35);
    color: #30d158;
    font-size: 0.72rem;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .auto-link-status-banner.error {
    background: rgba(255, 69, 58, 0.12);
    border-color: rgba(255, 69, 58, 0.35);
    color: #ff453a;
  }

  .status-dismiss-btn {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 0.95rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
    opacity: 0.8;
  }

  .status-dismiss-btn:hover {
    opacity: 1;
  }

  .spinner-inline {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-radius: 50%;
    border-top-color: #ffffff;
    animation: spin 0.8s linear infinite;
  }

  /* Markdown Table Rendering Styles */
  :global(.md-table-wrapper) {
    overflow-x: auto;
    margin: 10px 0;
    border-radius: 6px;
    border: 1px solid #33333d;
    background-color: #151518;
  }

  :global(.md-table) {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.78rem;
    text-align: left;
  }

  :global(.md-table th) {
    background-color: #22222a;
    color: #64d2ff;
    padding: 7px 12px;
    font-weight: 700;
    border-bottom: 1px solid #383844;
    letter-spacing: 0.02em;
  }

  :global(.md-table td) {
    padding: 7px 12px;
    border-bottom: 1px solid #24242c;
    color: #e4e4e7;
  }

  :global(.md-table tr:last-child td) {
    border-bottom: none;
  }

  :global(.md-table tr:nth-child(even)) {
    background-color: rgba(255, 255, 255, 0.02);
  }

  :global(.md-table tr:hover) {
    background-color: rgba(100, 210, 255, 0.04);
  }

  /* Playlist Sidebar Enhancements (Unlinked / Singer Subheads / Toolbar) */
  .playlist-item-sidebar.is-unlinked {
    border-left: 2px solid #ff9500;
    opacity: 0.85;
  }

  .playlist-item-text-stack {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }

  .playlist-item-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .unlinked-pill {
    background: rgba(255, 149, 0, 0.2);
    border: 1px solid #ff9500;
    color: #ff9500;
    font-size: 0.6rem;
    font-weight: 700;
    text-transform: uppercase;
    padding: 1px 4px;
    border-radius: 3px;
    letter-spacing: 0.03em;
  }

  .item-singer-subhead {
    font-size: 0.68rem;
    color: #a1a1aa;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .setlist-ai-btn {
    background: linear-gradient(135deg, #2c1a4d, #1c1c28);
    border-color: #bf5af2;
    color: #e599f7;
    font-weight: 700;
  }

  .setlist-ai-btn:hover {
    background: linear-gradient(135deg, #442277, #282838);
    color: #ffffff;
    border-color: #d0bfff;
  }

  .repair-btn {
    background: #242430;
    border-color: #4a4a58;
    color: #64d2ff;
  }

  .repair-btn:hover {
    background: #343444;
    color: #ffffff;
  }

  .missing-count-badge {
    color: #ff453a;
    font-weight: 800;
  }

  /* Setlist Importer Modal */
  .setlist-modal-card {
    min-width: 550px;
    max-width: 96vw;
    min-height: 400px;
    max-height: 94vh;
    display: flex;
    flex-direction: column;
    position: relative;
    box-sizing: border-box;
  }

  .setlist-modal-card.is-resizing {
    user-select: none !important;
  }

  .modal-header-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .modal-icon-btn {
    background: transparent;
    border: none;
    color: #8e8e93;
    font-size: 0.9rem;
    padding: 2px 6px;
    cursor: pointer;
    border-radius: 4px;
    transition: all 0.12s ease;
  }

  .modal-icon-btn:hover {
    background-color: #2b2b32;
    color: #ffffff;
  }

  .setlist-modal-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    position: relative;
  }

  .footer-left-info {
    display: flex;
    align-items: center;
  }

  .resize-hint-text {
    font-size: 0.65rem;
    color: #636366;
    font-family: monospace;
    user-select: none;
  }

  .footer-actions-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .modal-corner-resizer {
    position: absolute;
    right: 3px;
    bottom: 3px;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    opacity: 0.5;
    transition: opacity 0.15s ease;
    user-select: none;
  }

  .modal-corner-resizer:hover {
    opacity: 1.0;
  }

  .ai-badge {
    background: #bf5af2;
    color: #ffffff;
  }

  .setlist-modal-body {
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
    flex-grow: 1;
    min-height: 0;
  }

  .setlist-input-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }

  .setlist-header-actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .mini-sample-btn {
    background: #2a2a34;
    border: 1px solid #3e3e4e;
    color: #64d2ff;
    font-size: 0.7rem;
    font-weight: 600;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .mini-sample-btn.load-file-btn {
    background: linear-gradient(135deg, #2c1a4d, #1c1c28);
    border-color: #bf5af2;
    color: #e599f7;
    font-weight: 700;
  }

  .mini-sample-btn.load-file-btn:hover {
    background: linear-gradient(135deg, #442277, #282838);
    border-color: #d0bfff;
    color: #ffffff;
  }

  .mini-sample-btn:hover {
    background: #383848;
    color: #ffffff;
  }

  .setlist-csv-textarea {
    width: 100%;
    height: 110px;
    box-sizing: border-box;
    background: #141418;
    border: 1px solid #33333d;
    border-radius: 5px;
    color: #e4e4e7;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-size: 0.76rem;
    padding: 8px 10px;
    resize: vertical;
    outline: none;
    transition: border-color 0.12s ease;
  }

  .setlist-csv-textarea:focus {
    border-color: #bf5af2;
    box-shadow: 0 0 0 1px rgba(191, 90, 242, 0.3);
  }

  .setlist-action-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 8px;
  }

  .resolve-btn {
    background: #bf5af2;
    color: #ffffff;
    font-weight: 700;
  }

  .resolve-btn:hover:not(:disabled) {
    background: #cf70fa;
  }

  .setlist-error-banner {
    background: rgba(255, 69, 58, 0.15);
    border: 1px solid #ff453a;
    color: #ff453a;
    padding: 8px 12px;
    border-radius: 4px;
    font-size: 0.78rem;
  }

  .setlist-results-section {
    display: flex;
    flex-direction: column;
    flex-grow: 1;
    min-height: 0;
  }

  .setlist-table-scroll {
    flex-grow: 1;
    min-height: 180px;
    max-height: 55vh;
    overflow-y: auto;
    border: 1px solid #2e2e38;
    border-radius: 5px;
    background: #15151a;
  }

  .setlist-preview-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.74rem;
    text-align: left;
  }

  .setlist-preview-table th {
    background: #202028;
    color: #a1a1aa;
    padding: 6px 10px;
    font-weight: 700;
    border-bottom: 1px solid #33333f;
    position: sticky;
    top: 0;
    z-index: 1;
  }

  .setlist-preview-table td {
    padding: 6px 10px;
    border-bottom: 1px solid #22222b;
    color: #d1d1d6;
  }

  .setlist-preview-table tr.row-unlinked {
    background: rgba(255, 149, 0, 0.04);
  }

  .file-match-cell {
    max-width: 130px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-size: 0.7rem;
    color: #64d2ff;
  }

  .match-badge {
    display: inline-block;
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.65rem;
    font-weight: 700;
    text-transform: uppercase;
  }

  .match-ok {
    background: rgba(48, 209, 88, 0.2);
    color: #30d158;
    border: 1px solid #30d158;
  }

  .match-fuzzy {
    background: rgba(255, 214, 10, 0.2);
    color: #ffd60a;
    border: 1px solid #ffd60a;
  }

  .match-unlinked {
    background: rgba(255, 149, 0, 0.2);
    color: #ff9500;
    border: 1px solid #ff9500;
  }

  /* Playlist Repair Modal */
  .repair-modal-card {
    width: 680px;
    max-width: 95vw;
    max-height: 85vh;
  }

  .repair-badge {
    background: #ff9500;
    color: #ffffff;
  }

  .repair-modal-body {
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
  }

  .repair-items-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 380px;
    overflow-y: auto;
  }

  .repair-item-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: #191920;
    border: 1px solid #2c2c36;
    border-radius: 6px;
  }

  .repair-item-row.has-issue {
    border-color: #ff9500;
    background: #201a14;
  }

  .repair-item-left {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }

  .repair-status-icon {
    font-size: 1.1rem;
  }

  .repair-item-details {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .repair-track-name {
    font-size: 0.8rem;
    font-weight: 700;
    color: #f2f2f7;
  }

  .repair-track-path {
    font-size: 0.68rem;
    color: #8e8e93;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .repair-suggestion-box {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
    padding: 3px 6px;
    background: rgba(100, 210, 255, 0.1);
    border: 1px solid rgba(100, 210, 255, 0.3);
    border-radius: 4px;
    font-size: 0.72rem;
  }

  .suggest-label {
    color: #64d2ff;
    font-weight: 600;
  }

  .suggest-name {
    color: #ffffff;
    font-weight: 700;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
  }

  .suggest-conf {
    color: #a1a1aa;
    font-size: 0.66rem;
  }

  .health-badge {
    display: inline-block;
    padding: 3px 8px;
    border-radius: 4px;
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .health-ok {
    background: rgba(48, 209, 88, 0.2);
    color: #30d158;
    border: 1px solid #30d158;
  }

  .health-upgrade {
    background: rgba(100, 210, 255, 0.2);
    color: #64d2ff;
    border: 1px solid #64d2ff;
  }

  .health-missing {
    background: rgba(255, 69, 58, 0.2);
    color: #ff453a;
    border: 1px solid #ff453a;
  }

  /* Selective Migration Section in Repair Modal */
  .repair-migration-section {
    margin-top: 10px;
    padding: 12px 14px;
    background: #141419;
    border: 1px solid #282832;
    border-radius: 6px;
  }

  .repair-migration-title {
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #ffd60a;
    margin-bottom: 10px;
    text-transform: uppercase;
  }

  .repair-migration-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 8px 12px;
  }

  .repair-check-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.75rem;
    color: #e4e4e7;
    cursor: pointer;
    user-select: none;
  }

  .repair-check-label input[type="checkbox"] {
    accent-color: #ffd60a;
    width: 14px;
    height: 14px;
    cursor: pointer;
  }

  /* Volume Envelope Strip (Below Main Waveform) */
  .envelope-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 14px;
    background: #111116;
    border-top: 1px solid #23232c;
    border-bottom: 1px solid #23232c;
    font-size: 0.74rem;
    color: #d1d1d6;
    min-height: 34px;
    box-sizing: border-box;
  }

  .envelope-strip-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .envelope-toggle-btn {
    background: #1d1d24;
    border: 1px solid #383844;
    color: #ffb340;
    font-size: 0.72rem;
    font-weight: 700;
    padding: 3px 9px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .envelope-toggle-btn.active {
    background: rgba(255, 179, 64, 0.2);
    border-color: #ffb340;
    color: #ffffff;
  }

  .envelope-state-btn {
    background: rgba(48, 209, 88, 0.15);
    border: 1px solid #30d158;
    color: #30d158;
    font-size: 0.65rem;
    font-weight: 800;
    padding: 2px 7px;
    border-radius: 3px;
    cursor: pointer;
  }

  .envelope-state-btn.bypassed {
    background: rgba(255, 69, 58, 0.15);
    border-color: #ff453a;
    color: #ff453a;
  }

  .envelope-reset-btn {
    background: #202028;
    border: 1px solid #333340;
    color: #a1a1aa;
    font-size: 0.7rem;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
  }

  .envelope-reset-btn:hover:not(:disabled) {
    background: #2a2a36;
    color: #ffffff;
  }

  .envelope-reset-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .envelope-node-count-badge {
    font-size: 0.68rem;
    color: #8e8e93;
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
  }

  .envelope-strip-right {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .envelope-node-editor {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #181820;
    padding: 2px 8px;
    border: 1px solid #323240;
    border-radius: 4px;
  }

  .env-editor-label {
    font-weight: 700;
    color: #ffb340;
    font-size: 0.7rem;
  }

  .env-time-label {
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-size: 0.72rem;
    color: #64d2ff;
  }

  .env-gain-control {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .env-gain-label {
    font-family: -apple-system, BlinkMacSystemFont, "SF Mono", monospace;
    font-weight: 700;
    font-size: 0.74rem;
    color: #ffd60a;
    min-width: 54px;
  }

  .env-gain-slider {
    width: 80px;
    height: 4px;
    accent-color: #ffd60a;
    cursor: pointer;
  }

  .env-curve-selector {
    display: flex;
    gap: 2px;
    background: #101014;
    padding: 2px;
    border-radius: 4px;
  }

  .env-curve-btn {
    background: transparent;
    border: none;
    color: #8e8e93;
    font-size: 0.65rem;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
  }

  .env-curve-btn.active {
    background: #ffb340;
    color: #000000;
    font-weight: 700;
  }

  .env-delete-node-btn {
    background: transparent;
    border: none;
    color: #ff453a;
    font-size: 0.85rem;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 3px;
  }

  .env-delete-node-btn:hover {
    background: rgba(255, 69, 58, 0.2);
  }

  .env-hint-text {
    font-size: 0.7rem;
    color: #71717a;
    font-style: italic;
  }

  /* About Modal Styles */
  .about-modal-card {
    max-width: 440px;
    width: 90%;
  }

  .about-badge {
    background: #0071e3;
    color: #ffffff;
    font-weight: 700;
  }

  .about-modal-body {
    padding: 24px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .about-hero {
    text-align: center;
    padding-bottom: 12px;
    border-bottom: 1px solid #27272a;
  }

  .about-app-title {
    font-size: 1.5rem;
    font-weight: 800;
    letter-spacing: -0.02em;
    color: #f4f4f5;
  }

  .about-version-tag {
    display: inline-block;
    margin-top: 4px;
    padding: 2px 10px;
    background: #27272a;
    border-radius: 12px;
    font-size: 0.8rem;
    font-weight: 600;
    color: #38bdf8;
    border: 1px solid #38bdf840;
  }

  .about-build-date {
    margin-top: 8px;
    font-size: 0.72rem;
    color: #a1a1aa;
  }

  .about-info-grid {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: #141418;
    padding: 12px 14px;
    border-radius: 6px;
    border: 1px solid #27272a;
  }

  .about-info-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.76rem;
  }

  .about-info-label {
    color: #71717a;
    font-weight: 500;
  }

  .about-info-val {
    color: #e4e4e7;
    font-weight: 600;
  }

  .about-modal-footer {
    display: flex;
    justify-content: flex-end;
    padding: 12px 20px 16px;
    border-top: 1px solid #27272a;
  }
</style>
