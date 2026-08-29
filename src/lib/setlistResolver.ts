// Smart Setlist CSV Parser, Fuzzy Asset Resolver, and AI LLM Matching Engine

export interface SetlistRow {
  order: number;
  title: string;
  singers: string;
  keyNote: string;
  notes: string;
  rawRow: Record<string, string>;
}

export interface ScannedAsset {
  name: string;
  path: string;
  relative_path: string;
  extension: string;
  size_bytes: number;
  mtime_ms: number;
}

export interface ResolvedSetlistItem {
  order: number;
  title: string;
  singers: string;
  keyNote: string;
  notes: string;
  notesMarkdown: string;

  // Matched file assets
  mainAacPath?: string;
  mainAacName?: string;

  fullResWavPath?: string;
  fullResWavName?: string;

  originalPath?: string;
  originalName?: string;

  pdfPath?: string;
  pdfName?: string;

  isPlaceholder: boolean;
  confidence: "exact" | "high" | "fuzzy" | "unlinked";
}

/**
 * Robust CSV / TSV text parser with automatic delimiter and header detection.
 */
export function parseSetlistCsv(rawText: string): SetlistRow[] {
  if (!rawText || !rawText.trim()) return [];

  const lines = rawText
    .split(/\r?\n/)
    .map(l => l.trim())
    .filter(l => l.length > 0);

  if (lines.length === 0) return [];

  // Determine delimiter: tab, comma, or semicolon
  const firstLine = lines[0];
  let delimiter = ",";
  if (firstLine.includes("\t")) delimiter = "\t";
  else if (firstLine.includes(";") && !firstLine.includes(",")) delimiter = ";";

  const parseLine = (line: string): string[] => {
    if (delimiter === "\t") return line.split("\t").map(s => s.trim().replace(/^["']|["']$/g, ""));
    // Parse comma/semicolon respecting quotes
    const result: string[] = [];
    let cur = "";
    let inQuotes = false;
    for (let i = 0; i < line.length; i++) {
      const char = line[i];
      if (char === '"' || char === "'") {
        inQuotes = !inQuotes;
      } else if (char === delimiter && !inQuotes) {
        result.push(cur.trim().replace(/^["']|["']$/g, ""));
        cur = "";
      } else {
        cur += char;
      }
    }
    result.push(cur.trim().replace(/^["']|["']$/g, ""));
    return result;
  };

  const headers = parseLine(lines[0]).map(h => h.toLowerCase().trim());

  // Check if first line is a recognized header row
  const titleKeywords = ["song", "title", "name", "track", "piece", "tune"];
  const singerKeywords = ["singer", "singers", "vocal", "vocals", "vocalist", "vocalists", "lead", "performer", "artist"];
  const keyKeywords = ["key", "tonality", "pitch", "root"];
  const notesKeywords = ["note", "notes", "cue", "cues", "arrangement", "info", "comment", "comments", "details"];
  const orderKeywords = ["order", "#", "no", "no.", "num", "number", "index", "pos", "position"];

  let titleCol = headers.findIndex(h => titleKeywords.some(k => h.includes(k)));
  let singerCol = headers.findIndex(h => singerKeywords.some(k => h.includes(k)));
  let keyCol = headers.findIndex(h => keyKeywords.some(k => h.includes(k)));
  let notesCol = headers.findIndex(h => notesKeywords.some(k => h.includes(k)));
  let orderCol = headers.findIndex(h => orderKeywords.some(k => h === k || h.startsWith("#")));

  let dataLines = lines.slice(1);

  // If no header row detected, assume line 0 is data
  if (titleCol === -1) {
    titleCol = 0;
    singerCol = headers.length > 1 ? 1 : -1;
    keyCol = headers.length > 2 ? 2 : -1;
    notesCol = headers.length > 3 ? 3 : -1;
    orderCol = -1;
    dataLines = lines;
  }

  const rows: SetlistRow[] = [];

  dataLines.forEach((line, idx) => {
    const cols = parseLine(line);
    if (cols.length === 0 || cols.every(c => !c)) return;

    const title = (titleCol !== -1 && cols[titleCol]) ? cols[titleCol] : cols[0] || `Track ${idx + 1}`;
    if (!title) return;

    const singers = singerCol !== -1 && cols[singerCol] ? cols[singerCol] : "";
    const keyNote = keyCol !== -1 && cols[keyCol] ? cols[keyCol] : "";
    const notes = notesCol !== -1 && cols[notesCol] ? cols[notesCol] : "";
    const parsedOrder = orderCol !== -1 && cols[orderCol] ? parseInt(cols[orderCol], 10) : (idx + 1);
    const order = isNaN(parsedOrder) ? (idx + 1) : parsedOrder;

    const rawRow: Record<string, string> = {};
    cols.forEach((val, cIdx) => {
      const headerName = headers[cIdx] || `Col_${cIdx + 1}`;
      rawRow[headerName] = val;
    });

    rows.push({
      order,
      title,
      singers,
      keyNote,
      notes,
      rawRow
    });
  });

  rows.sort((a, b) => a.order - b.order);
  return rows;
}

/**
 * Normalizes title string for fuzzy comparison.
 */
export function normalizeSongTitle(title: string): string {
  return title
    .toLowerCase()
    .replace(/\.[a-z0-9]{2,5}$/i, "") // remove extension
    .replace(/^[0-9]+[\s\-_.]+/, "") // remove leading track numbering "01 - " or "1. "
    .replace(/\(.*?\)|\[.*?\]/g, "") // remove bracketed content e.g. "(Live Backing)", "[Key of G]"
    .replace(/[_\-.]+/g, " ") // normalize dashes/underscores to spaces
    .replace(/\b(the|a|an)\b/g, "") // remove articles
    .replace(/[^a-z0-9\s]/g, "") // remove special characters
    .trim()
    .replace(/\s+/g, " ");
}

/**
 * Calculates token-based & string distance similarity score (0.0 to 1.0).
 */
export function calculateMatchScore(queryTitle: string, candidateFilename: string): number {
  const normQuery = normalizeSongTitle(queryTitle);
  const normCand = normalizeSongTitle(candidateFilename);

  if (!normQuery || !normCand) return 0;
  if (normQuery === normCand) return 1.0;
  if (normCand.includes(normQuery) || normQuery.includes(normCand)) return 0.9;

  // Word token overlap
  const queryTokens = new Set(normQuery.split(" ").filter(w => w.length > 1));
  const candTokens = new Set(normCand.split(" ").filter(w => w.length > 1));

  if (queryTokens.size === 0 || candTokens.size === 0) return 0;

  let intersectionCount = 0;
  queryTokens.forEach(t => {
    if (candTokens.has(t)) intersectionCount++;
  });

  const jaccard = intersectionCount / (queryTokens.size + candTokens.size - intersectionCount);

  // Levenshtein similarity
  const len = Math.max(normQuery.length, normCand.length);
  const levDist = levenshteinDistance(normQuery, normCand);
  const levScore = Math.max(0, 1.0 - levDist / len);

  return Math.max(jaccard, levScore);
}

function levenshteinDistance(a: string, b: string): number {
  const matrix: number[][] = [];
  for (let i = 0; i <= b.length; i++) matrix[i] = [i];
  for (let j = 0; j <= a.length; j++) matrix[0][j] = j;

  for (let i = 1; i <= b.length; i++) {
    for (let j = 1; j <= a.length; j++) {
      if (b.charAt(i - 1) === a.charAt(j - 1)) {
        matrix[i][j] = matrix[i - 1][j - 1];
      } else {
        matrix[i][j] = Math.min(
          matrix[i - 1][j - 1] + 1,
          Math.min(matrix[i][j - 1] + 1, matrix[i - 1][j] + 1)
        );
      }
    }
  }
  return matrix[b.length][a.length];
}

/**
 * Finds best matching file from scanned library files.
 */
export function findBestMatch(
  songTitle: string,
  candidates: ScannedAsset[],
  minThreshold = 0.55
): { asset: ScannedAsset; score: number } | null {
  if (!candidates || candidates.length === 0) return null;

  let bestAsset: ScannedAsset | null = null;
  let bestScore = 0;

  for (const cand of candidates) {
    const score = calculateMatchScore(songTitle, cand.name);
    if (score > bestScore && score >= minThreshold) {
      bestScore = score;
      bestAsset = cand;
    } else if (score === bestScore && bestAsset) {
      // Tie breaker: prefer newer modification time or higher version tag
      if (cand.mtime_ms > bestAsset.mtime_ms) {
        bestAsset = cand;
      }
    }
  }

  return bestAsset ? { asset: bestAsset, score: bestScore } : null;
}

/**
 * Formats singer assignments, key, and arrangement details into a clean Markdown table.
 */
export function generateSingerMarkdown(row: SetlistRow): string {
  let md = `### Rehearsal & Performance Info\n\n`;
  md += `| Attribute | Details |\n`;
  md += `| :--- | :--- |\n`;
  md += `| **Song Title** | ${row.title} |\n`;
  if (row.singers) {
    md += `| **Lead Singer(s)** | ${row.singers} |\n`;
  }
  if (row.keyNote) {
    md += `| **Key / Pitch** | [${row.keyNote}] |\n`;
  }
  if (row.notes) {
    md += `| **Arrangement Cues** | ${row.notes} |\n`;
  }
  md += `| **Set Order** | Track #${row.order} |\n`;

  return md;
}

/**
 * Resolves full setlist using local fast fuzzy matching engine.
 */
export function resolveSetlistLocal(
  rows: SetlistRow[],
  libraries: {
    aacFiles: ScannedAsset[];
    hiresFiles: ScannedAsset[];
    origFiles: ScannedAsset[];
    pdfFiles: ScannedAsset[];
  }
): ResolvedSetlistItem[] {
  return rows.map((row) => {
    const aacMatch = findBestMatch(row.title, libraries.aacFiles);
    const hiresMatch = findBestMatch(row.title, libraries.hiresFiles);
    const origMatch = findBestMatch(row.title, libraries.origFiles);
    const pdfMatch = findBestMatch(row.title, libraries.pdfFiles);

    const isPlaceholder = !aacMatch && !hiresMatch;
    let confidence: "exact" | "high" | "fuzzy" | "unlinked" = "unlinked";

    if (aacMatch) {
      if (aacMatch.score >= 0.9) confidence = "exact";
      else if (aacMatch.score >= 0.75) confidence = "high";
      else confidence = "fuzzy";
    } else if (hiresMatch) {
      confidence = "high";
    }

    return {
      order: row.order,
      title: row.title,
      singers: row.singers,
      keyNote: row.keyNote,
      notes: row.notes,
      notesMarkdown: generateSingerMarkdown(row),

      mainAacPath: aacMatch?.asset.path || hiresMatch?.asset.path,
      mainAacName: aacMatch?.asset.name || hiresMatch?.asset.name,

      fullResWavPath: hiresMatch?.asset.path,
      fullResWavName: hiresMatch?.asset.name,

      originalPath: origMatch?.asset.path,
      originalName: origMatch?.asset.name,

      pdfPath: pdfMatch?.asset.path,
      pdfName: pdfMatch?.asset.name,

      isPlaceholder,
      confidence
    };
  });
}
