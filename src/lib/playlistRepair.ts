// Playlist Health & Missing File Replacement Engine

import { type ScannedAsset, calculateMatchScore, findBestMatch } from "./setlistResolver";

export interface PlaylistItemHealth {
  index: number;
  originalName: string;
  originalPath: string;
  exists: boolean;
  isPlaceholder: boolean;

  suggestedReplacement?: ScannedAsset;
  replacementConfidence?: "exact" | "high" | "fuzzy";
}

/**
 * Evaluates the health of each track in the playlist against actual disk files.
 * Identifies missing audio files and suggests newer mix versions from library folders.
 */
export function analyzePlaylistHealth(
  items: Array<{ name: string; path: string; isPlaceholder?: boolean }>,
  fileExistenceMap: boolean[],
  availableLibraryFiles: ScannedAsset[]
): PlaylistItemHealth[] {
  return items.map((item, index) => {
    const exists = fileExistenceMap[index] ?? false;
    const isPlaceholder = item.isPlaceholder || !item.path;

    if (exists && !isPlaceholder) {
      return {
        index,
        originalName: item.name,
        originalPath: item.path,
        exists: true,
        isPlaceholder: false
      };
    }

    // File is missing or unlinked placeholder -> Search library folders for replacement / upgrade
    const match = findBestMatch(item.name, availableLibraryFiles, 0.5);

    let replacementConfidence: "exact" | "high" | "fuzzy" | undefined = undefined;
    if (match) {
      if (match.score >= 0.9) replacementConfidence = "exact";
      else if (match.score >= 0.7) replacementConfidence = "high";
      else replacementConfidence = "fuzzy";
    }

    return {
      index,
      originalName: item.name,
      originalPath: item.path,
      exists,
      isPlaceholder,
      suggestedReplacement: match?.asset,
      replacementConfidence
    };
  });
}
