// Audio file tags system (auto-detected and user-toggled via right-click)

export interface AudioTagOption {
  id: string;
  label: string;
  color: string;
}

export const AUDIO_TAG_OPTIONS: AudioTagOption[] = [
  { id: "original", label: "Original", color: "#d084ff" },
  { id: "vocals_only", label: "Vocals only", color: "#30d158" },
  { id: "lead_vocal", label: "Lead Vocal", color: "#ffd60a" },
  { id: "background_vocals", label: "Background Vocals", color: "#64d2ff" },
  { id: "iso_track", label: "Iso Track", color: "#38bdf8" },
  { id: "track", label: "Track", color: "#ff9f0a" }
];

export function getFileExtension(pathOrName: string): string {
  if (!pathOrName) return "";
  const clean = pathOrName.split("?")[0].split("#")[0];
  const dotIdx = clean.lastIndexOf(".");
  return dotIdx !== -1 ? clean.substring(dotIdx + 1).toUpperCase() : "";
}

export function isLosslessAudio(pathOrName: string): boolean {
  const ext = getFileExtension(pathOrName).toLowerCase();
  return ["wav", "aiff", "aif", "flac", "alac"].includes(ext);
}

export function isAudioFile(pathOrName: string): boolean {
  const ext = getFileExtension(pathOrName).toLowerCase();
  return ["wav", "mp3", "m4a", "aac", "flac", "aif", "aiff", "alac", "ogg", "wma"].includes(ext);
}

export function getAutoDetectedTags(path: string, name?: string, roleHint?: string, primarySongTrackPath?: string): string[] {
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
