/**
 * Pure routing/merge logic for the unified profile importer. Kept separate
 * from `import-profile-dialog.tsx` so the "what do we do with these dropped
 * or picked paths" decision is unit-testable without Tauri or React.
 */

export type ImportSourceKind =
  | "archive"
  | "profile_folder"
  | "data_directory"
  | "unknown";

/**
 * Mirrors the Rust `ImportSourceInfo` enum (`inspect_import_source`). An
 * archive is always a bundle of 1..N profiles — `profile_count`/
 * `preview_names` are only populated when it isn't password-protected
 * (reading a bundle's contents requires the password otherwise).
 */
export interface ImportSourceInfo {
  kind: ImportSourceKind;
  needs_password?: boolean;
  profile_count?: number | null;
  preview_names?: string[];
}

export interface ClassifiedPath {
  path: string;
  info: ImportSourceInfo;
}

export type ImportRoute =
  | { type: "archive_batch"; paths: ClassifiedPath[] }
  | { type: "profile_folder"; path: string }
  | { type: "data_directory"; path: string }
  | { type: "mixed_error" }
  | { type: "unknown_error" };

/**
 * Decide what to do with a set of classified paths dropped or picked in one
 * go. Archives are the only kind that batches — a folder or data directory
 * only makes sense one at a time, since each is itself a whole import unit.
 */
export function routeImportSources(classified: ClassifiedPath[]): ImportRoute {
  if (classified.length === 0) return { type: "unknown_error" };

  const kinds = new Set(classified.map((c) => c.info.kind));
  if (kinds.size === 1 && kinds.has("archive")) {
    return { type: "archive_batch", paths: classified };
  }

  if (classified.length === 1) {
    const [only] = classified;
    if (only.info.kind === "profile_folder") {
      return { type: "profile_folder", path: only.path };
    }
    if (only.info.kind === "data_directory") {
      return { type: "data_directory", path: only.path };
    }
  }

  return classified.length > 1
    ? { type: "mixed_error" }
    : { type: "unknown_error" };
}

export function basenameOf(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export type ArchiveBatchStatus = "pending" | "importing" | "success" | "error";

export interface ArchiveBatchItem {
  path: string;
  displayName: string;
  /** Profile names inside this archive (a bundle of 1..N), when known. */
  previewNames?: string[];
  needsPassword: boolean;
  password: string;
  status: ArchiveBatchStatus;
  errorMessage?: string;
  /** Names of the profiles actually created, once imported. */
  resultNames?: string[];
}

/**
 * Add newly classified archive paths to an existing batch, skipping any path
 * already present (dropping the same file twice, or dropping more files onto
 * an already-populated batch, must not duplicate rows or reset progress on
 * items already imported).
 */
export function mergeArchiveBatchItems(
  existing: ArchiveBatchItem[],
  additions: ClassifiedPath[],
): ArchiveBatchItem[] {
  const existingPaths = new Set(existing.map((item) => item.path));
  const newItems: ArchiveBatchItem[] = additions
    .filter((a) => a.info.kind === "archive" && !existingPaths.has(a.path))
    .map((a) => ({
      path: a.path,
      displayName: basenameOf(a.path),
      previewNames: a.info.preview_names,
      needsPassword: Boolean(a.info.needs_password),
      password: "",
      status: "pending",
    }));
  return [...existing, ...newItems];
}
