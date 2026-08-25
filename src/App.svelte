<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import * as pdfjsLib from "pdfjs-dist";
  import pdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

  pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorker;

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
  let eqBass = 0;                   // Low Shelf: 100 Hz (-12 - +12 dB)
  let eqMid = 0;                    // Mid Parametric Bell: 1 kHz (-12 - +12 dB)
  let eqTreble = 0;                 // High Shelf: 8 kHz (-12 - +12 dB)
  let compressorThreshold = -20;    // range -60 - 0 dB
  let compressorRatio = 2.0;        // range 1.0 - 20.0 (Ratio)
  let compressorMakeup = 0;         // range 0 - 24 dB
  
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
  let pdfChartPath = "";
  let pdfChartName = "";
  let associatedVersions: PlaylistItem[] = [];

  function switchCenterTab(tab: "notes" | "lyrics" | "metadata" | "pdf") {
    activeCenterTab = tab;
    lastScrolledMarkerId = null;
    localStorage.setItem("th_last_center_tab", tab);
    saveCurrentTrackProfile(filePath);
    if (tab === "pdf") {
      tick().then(() => renderPdfMarkerBadges());
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
    compressorThreshold: number;
    compressorRatio: number;
    compressorMakeup: number;
    markers: Marker[];
    nextMarkerId: number;
    pdfChartPath: string;
    pdfChartName: string;
    associatedVersions: PlaylistItem[];
    alternateTrackPath?: string | null;
    notes?: string;
    lyrics?: string;
    lastCenterTab?: "notes" | "lyrics" | "metadata" | "pdf";
  }

  function getProfilesStore(): Record<string, TrackProfile> {
    try {
      const data = localStorage.getItem("th_track_profiles");
      return data ? JSON.parse(data) : {};
    } catch {
      return {};
    }
  }

  function saveCurrentTrackProfile(trackPath: string | null) {
    if (!trackPath) return;
    const store = getProfilesStore();
    const existing = store[trackPath] || {};
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
      compressorThreshold,
      compressorRatio,
      compressorMakeup,
      markers: JSON.parse(JSON.stringify(markers)),
      nextMarkerId,
      pdfChartPath: isMain ? pdfChartPath : (existing.pdfChartPath || ""),
      pdfChartName: isMain ? pdfChartName : (existing.pdfChartName || ""),
      associatedVersions: isMain ? associatedVersions : (existing.associatedVersions || []),
      alternateTrackPath: isMain ? (alternateTrack ? alternateTrack.path : null) : (existing.alternateTrackPath || null),
      notes: isMain ? songNotes : (existing.notes || ""),
      lyrics: isMain ? songLyrics : (existing.lyrics || ""),
      lastCenterTab: activeCenterTab
    };
    localStorage.setItem("th_track_profiles", JSON.stringify(store));
  }

  async function updatePitchEngine() {
    const totalSemitones = pitch + (pitchCents / 100.0);
    await invoke("set_pitch", { pitch: totalSemitones });
    saveCurrentTrackProfile(filePath);
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

  // In-Tab PDF Viewer State
  let pdfContainer: HTMLDivElement;
  let isLoadingPdf = false;
  let pdfTotalPages = 0;
  let pdfCurrentPage = 1;
  let isPdfInverted = false;
  let pdfRenderError = "";
  let currentRenderTaskId = 0;
  let lastRenderedWidth = 0;

  function togglePdfInvert() {
    isPdfInverted = !isPdfInverted;
    localStorage.setItem("th_pdf_inverted", isPdfInverted ? "1" : "0");
    if (pdfContainer) {
      const cards = pdfContainer.querySelectorAll(".pdf-page-card");
      cards.forEach(c => {
        if (isPdfInverted) {
          c.classList.add("inverted");
        } else {
          c.classList.remove("inverted");
        }
      });
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
      compressorThreshold = typeof profile.compressorThreshold === "number" ? profile.compressorThreshold : -20;
      compressorRatio = typeof profile.compressorRatio === "number" ? profile.compressorRatio : 2.0;
      compressorMakeup = typeof profile.compressorMakeup === "number" ? profile.compressorMakeup : 0;
      markers = Array.isArray(profile.markers) ? [...profile.markers] : [];
      nextMarkerId = typeof profile.nextMarkerId === "number" ? profile.nextMarkerId : (markers.length + 1);

      if (isMainSong) {
        pdfChartPath = profile.pdfChartPath || "";
        pdfChartName = profile.pdfChartName || (pdfChartPath ? (pdfChartPath.split("/").pop() || "") : "");
        associatedVersions = Array.isArray(profile.associatedVersions) ? profile.associatedVersions : [];
        songNotes = profile.notes || "";
        songLyrics = profile.lyrics || "";
        if (profile.lastCenterTab) {
          activeCenterTab = profile.lastCenterTab;
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
      eqBass = 0;
      eqMid = 0;
      eqTreble = 0;
      compressorThreshold = -20;
      compressorRatio = 2.0;
      compressorMakeup = 0;
      markers = [];
      nextMarkerId = 1;

      if (isMainSong) {
        pdfChartPath = "";
        pdfChartName = "";
        associatedVersions = [];
        songNotes = "";
        songLyrics = "";
        alternateTrack = null;

        const savedTab = localStorage.getItem("th_last_center_tab") as "notes" | "lyrics" | "metadata" | "pdf" | null;
        if (savedTab) {
          activeCenterTab = savedTab;
        }
      }
    }

    // Apply restored volume, speed, and pitch directly to audio engine
    const linearVol = dbVolume <= -59.5 ? 0 : Math.pow(10, dbVolume / 20);
    await invoke("set_volume", { volume: linearVol });
    await invoke("set_speed", { speed });
    const totalSemitones = pitch + (pitchCents / 100.0);
    await invoke("set_pitch", { pitch: totalSemitones });
  }

  // Canvas elements
  let mainCanvas: HTMLCanvasElement;
  let overviewCanvas: HTMLCanvasElement;
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
        if (!isPanning && !isDraggingOverview) {
          currentTime = status.current_time;
          duration = status.duration_seconds;
          progress = status.progress;
          
          await updateVisiblePeaks();

          drawMainWaveform();
          drawOverviewWaveform();

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
        drawMainWaveform();
        drawOverviewWaveform();
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

    // Close context menu & color palette on window click
    const closeMenu = () => { 
      showContextMenu = false; 
      colorPaletteMarker = null;
    };
    window.addEventListener("click", closeMenu);

    // Global keyboard shortcuts: Space = Play/Pause/Load, Up/Down = Playlist/Browser Nav, Left/Right = Marker Jump, M = Add Marker, Enter = Stop & Return to 0
    const handleKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }

      // If active type-to-jump buffer is receiving input
      if (typeToJumpBuffer.length > 0) {
        if (e.code === "Space") {
          e.preventDefault();
          typeToJumpBuffer += " ";
          clearTimeout(typeToJumpTimeout);
          typeToJumpTimeout = setTimeout(() => { typeToJumpBuffer = ""; }, 1000);
          clearTimeout(jumpDebounceTimeout);
          jumpDebounceTimeout = setTimeout(() => { performTypeToJump(); }, 300);
          return;
        }
      }

      if (e.code === "Space") {
        e.preventDefault();
        
        // Option 2: If actively navigating playlist and a different track is highlighted, load and immediately play!
        if (activeSidebarTab === "playlist" && selectedPlaylistIndex >= 0 && selectedPlaylistIndex < playlistItems.length) {
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
        if (activeSidebarTab === "playlist" && playlistItems.length > 0) {
          let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.max(0, currentIdx - 1);
          selectedPlaylistIndex = currentIdx;
          scrollSelectedPlaylistItemIntoView();
        } else if (activeSidebarTab === "browser" && filteredEntries.length > 0) {
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
        if (activeSidebarTab === "playlist" && playlistItems.length > 0) {
          let currentIdx = selectedPlaylistIndex !== -1 ? selectedPlaylistIndex : playlistItems.findIndex(p => p.path === filePath);
          if (currentIdx === -1) currentIdx = 0;
          else currentIdx = Math.min(playlistItems.length - 1, currentIdx + 1);
          selectedPlaylistIndex = currentIdx;
          scrollSelectedPlaylistItemIntoView();
        } else if (activeSidebarTab === "browser" && filteredEntries.length > 0) {
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
        if (!e.metaKey && !e.ctrlKey && !e.altKey && typeToJumpBuffer.length === 0) {
          e.preventDefault();
          addMarker();
        }
      } else if (e.code === "ArrowLeft") {
        e.preventDefault();
        jumpToPrevMarker();
      } else if (e.code === "ArrowRight") {
        e.preventDefault();
        jumpToNextMarker();
      } else if (e.code === "Enter" || e.code === "NumpadEnter") {
        e.preventDefault();
        handleStop();
      } else if (e.code === "Escape") {
        e.preventDefault();
        showContextMenu = false;
        colorPaletteMarker = null;
        cancelRenameMarker();
        typeToJumpBuffer = "";
        handleStop();
      }
    };
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      clearInterval(statusInterval);
      if (resizeObserver) resizeObserver.disconnect();
      unlistenDragDrop.then(fn => fn());
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

      // Preload candidates asynchronously without blocking the UI thread
      const store = getProfilesStore();
      for (const path of candidates) {
        if (!path) continue;
        
        // Background decode audio and compute peaks into memory cache
        if (!preloadedTrackMetadata.has(path)) {
          invoke("preload_track", { path }).then((meta: any) => {
            if (meta) {
              preloadedTrackMetadata.set(path, meta);
            }
          }).catch(() => {});
        }

        // Background preload PDF score sheet bytes if associated
        const prof = store[path];
        if (prof && prof.pdfChartPath) {
          invoke("read_file_bytes", { path: prof.pdfChartPath }).catch(() => {});
        }

        // Background preload audio tags
        invoke("read_audio_metadata", { path }).catch(() => {});
      }
    } finally {
      isPreloading = false;
    }
  }

  // Load a file into main or alternate track slots
  async function loadAudioPath(path: string, target: "main" | "alternate", switchActive = true) {
    try {
      const wasPlaying = isPlaying;

      // Save current track profile before switching
      if (filePath) {
        saveCurrentTrackProfile(filePath);
      }

      const metadata: any = await invoke("load_track", { path });
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

      if (switchActive || target === "main") {
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
    
    if (activeSidebarTab === "playlist" && playlistItems.length > 0) {
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

          invoke("get_raw_samples", { startFrame: fetchStart, count: fetchCount })
            .then((res: any) => {
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

  // Main Waveform: Mouse Drag to Pan, click to seek, and Ruler Marker Dragging
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
            startMarkerPointerDrag(e, m, false, true);
            return;
          }
        }
      }
    }

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
    drawMainWaveform();
  }

  function deleteMarker(id: number) {
    markers = markers.filter(m => m.id !== id);
    saveCurrentTrackProfile(filePath);
    drawMainWaveform();
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

    // 4. Mono Continuous Single Line Waveform
    const halfHeight = Math.floor(height / 2);
    const maxAmplitude = (height / 2) - rulerHeight - 4;

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

    if (visibleSamples.length > 0) {
      const numSamples = visibleSamples.length;
      const step = width / (numSamples - 1 || 1);

      // True Continuous Oscillating Waveform Line (Unified across all zoom levels)
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

        // Vertical Marker Line
        ctx.strokeStyle = color; 
        ctx.lineWidth = 1.5;
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
        ctx.fillStyle = color;
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
        
        // Dashed Ghost Vertical Marker Line
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
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

    // Draw Markers on Overview Waveform
    if (track && track.duration > 0 && markers.length > 0) {
      for (const marker of markers) {
        const markerPct = marker.time / track.duration;
        if (markerPct >= 0 && markerPct <= 1.0) {
          const markerX = Math.round(markerPct * width);
          const color = marker.color || "#ff9500";
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.moveTo(markerX + 0.5, 0);
          ctx.lineTo(markerX + 0.5, height);
          ctx.stroke();

          // Top flag cap
          ctx.fillStyle = color;
          ctx.fillRect(markerX - 1.5, 0, 3, 4);
        }
      }
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

<svelte:window on:keydown={handleBrowserKeydown} />

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
            class="deck-tab-btn pdf-tab-btn" 
            class:active={activeCenterTab === "pdf"} 
            on:click={() => switchCenterTab("pdf")}
            title={pdfChartPath || "Sheet Music / Lead Sheet"}
          >
            {pdfChartName || "SHEET MUSIC (PDF)"}
          </button>
        </div>

        <div class="deck-tab-content">
          {#if activeCenterTab === "notes"}
            <div class="tab-pane notes-pane">
              <textarea 
                class="rehearsal-textarea notes-textarea" 
                placeholder="Type rehearsal notes, arrangement cues, key changes, or performance reminders here (auto-saved with this song)..."
                bind:value={songNotes}
                on:input={() => saveCurrentTrackProfile(filePath)}
              ></textarea>
            </div>
          {:else if activeCenterTab === "lyrics"}
            <div class="tab-pane lyrics-pane">
              <textarea 
                class="rehearsal-textarea lyrics-textarea" 
                placeholder="Type or paste song lyrics, vocal harmony cues, or performance text here (auto-saved with this song)..."
                bind:value={songLyrics}
                on:input={() => saveCurrentTrackProfile(filePath)}
              ></textarea>
            </div>
          {:else if activeCenterTab === "metadata"}
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
                    <div class="spec-row">
                      <span class="spec-label">Sheet Music:</span>
                      <span class="spec-val">{pdfChartName || "None linked"}</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          {:else if activeCenterTab === "pdf"}
            <div class="tab-pane pdf-pane">
              {#if pdfChartPath}
                <div class="pdf-viewer-container">
                  <!-- Sleek floating overlay in corner -->
                  <div class="pdf-floating-controls">
                    {#if pdfTotalPages > 0}
                      <span class="pdf-page-pill">{pdfCurrentPage}/{pdfTotalPages}</span>
                    {/if}
                    <button 
                      class="pdf-mini-btn" 
                      class:active-toggle={isPdfInverted}
                      on:click={togglePdfInvert} 
                      title="Toggle Inverted / Negative Dark Mode (White notes on Black paper)"
                    >
                      {isPdfInverted ? "☀️ Normal" : "🌙 Invert"}
                    </button>
                    <button class="pdf-mini-btn" on:click={associatePdfChart} title="Change PDF file">
                      Change
                    </button>
                    <button class="pdf-mini-btn popout" on:click={openPdfInExternalViewer} title="Open in Floating Window">
                      ⤢ Float
                    </button>
                  </div>

                  {#if isLoadingPdf}
                    <div class="pdf-loading-overlay">
                      <span class="pdf-loading-spinner">⏳</span>
                      <span>Rendering sheet music...</span>
                    </div>
                  {/if}

                  {#if pdfRenderError}
                    <div class="pdf-error-card">
                      <span>⚠️ {pdfRenderError}</span>
                      <button class="retry-pdf-btn" on:click={renderPdfPages}>Retry</button>
                    </div>
                  {/if}

                  <!-- Single scrollable column of auto-scaled PDF page canvases -->
                  <div 
                    class="pdf-scroll-column" 
                    bind:this={pdfContainer} 
                    on:scroll={handlePdfScroll}
                    on:dragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; }}
                    on:drop={handlePdfContainerDrop}
                  ></div>
                </div>
              {:else}
                <div class="no-pdf-placeholder">
                  <span class="pdf-placeholder-icon">📄</span>
                  <p>No sheet music or lead sheet PDF linked to this song.</p>
                  <button class="link-pdf-btn" on:click={associatePdfChart}>
                    + Link PDF Sheet Music / Lead Sheet...
                  </button>
                </div>
              {/if}
            </div>
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
                style="border-left: 3px solid {marker.color || '#ff9500'};"
                on:mousedown={(e) => handleMarkerItemMouseDown(e, marker)}
                title="Click to seek • Double-click to rename • Drag to waveform or sheet music"
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
                    on:click={() => seekToMarker(marker.time)}
                    on:dblclick={() => startRenameMarker(marker)}
                    title="Click to seek • Double-click to rename • Drag to waveform or PDF"
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

          <!-- Shared Project Landmarks Bin: Markers created in other takes/versions not yet placed on this track -->
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
            on:dblclick={() => resetKnob("comp_thresh", -20, (v) => compressorThreshold = v)}
            title="Double-click resets to -20 dB"
          >
            <span class="knob-label">Threshold</span>
            <div class="knob-circle">
              <div class="knob-zero-tick"></div>
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorThreshold, -60, 0)}deg)"></div>
            </div>
            <span class="knob-value">{compressorThreshold} dB</span>
          </div>

          <!-- Ratio -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "comp_ratio", compressorRatio, 1.0, 20.0, 0.5, (v) => compressorRatio = v)}
            on:dblclick={() => resetKnob("comp_ratio", 2.0, (v) => compressorRatio = v)}
            title="Double-click resets to 2.0:1"
          >
            <span class="knob-label">Ratio</span>
            <div class="knob-circle">
              <div class="knob-zero-tick"></div>
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorRatio, 1.0, 20.0)}deg)"></div>
            </div>
            <span class="knob-value">{compressorRatio.toFixed(1)}:1</span>
          </div>

          <!-- Makeup -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "comp_makeup", compressorMakeup, 0, 24, 0.5, (v) => compressorMakeup = v)}
            on:dblclick={() => resetKnob("comp_makeup", 0, (v) => compressorMakeup = v)}
            title="Double-click resets to 0 dB"
          >
            <span class="knob-label">Makeup</span>
            <div class="knob-circle">
              <div class="knob-zero-tick"></div>
              <div class="knob-marker" style="transform: rotate({getKnobRotation(compressorMakeup, 0, 24)}deg)"></div>
            </div>
            <span class="knob-value">+{compressorMakeup.toFixed(1)} dB</span>
          </div>
        </div>

        <!-- 3. Equalizer knobs (Low 100Hz, Mid 1kHz, High 8kHz) -->
        <div class="knobs-row placeholder-knobs">
          <!-- Low Shelf (100 Hz) -->
          <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
          <div 
            class="knob-container" 
            on:mousedown={(e) => handleKnobMousedown(e, "eq_bass", eqBass, -12, 12, 0.5, (v) => eqBass = v)}
            on:dblclick={() => resetKnob("eq_bass", 0, (v) => eqBass = v)}
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
            on:mousedown={(e) => handleKnobMousedown(e, "eq_mid", eqMid, -12, 12, 0.5, (v) => eqMid = v)}
            on:dblclick={() => resetKnob("eq_mid", 0, (v) => eqMid = v)}
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
            on:mousedown={(e) => handleKnobMousedown(e, "eq_treble", eqTreble, -12, 12, 0.5, (v) => eqTreble = v)}
            on:dblclick={() => resetKnob("eq_treble", 0, (v) => eqTreble = v)}
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

        <!-- 2. Speed, Pitch, and Fine Tune Knobs (3 knobs on same line) -->
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
    display: flex;
    justify-content: space-around;
    padding: 3px 2px;
    background-color: #1e1e1f;
    border-bottom: 1px solid #2d2d2d;
  }

  .knob-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    cursor: ns-resize;
    width: 48px;
  }

  .knob-label {
    font-size: 0.58rem;
    color: #8e8e8e;
    margin-bottom: 1px;
    text-align: center;
    white-space: nowrap;
  }

  .knob-circle {
    width: 19px;
    height: 19px;
    border-radius: 50%;
    background-color: #333333;
    border: 1.5px solid #555555;
    position: relative;
    box-shadow: inset 0 2px 4px rgba(0,0,0,0.5);
    margin-bottom: 1px;
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
    height: 6px;
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
</style>
