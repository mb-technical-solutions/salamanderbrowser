"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaFolder } from "react-icons/fa";
import {
  LuChevronRight,
  LuCloudUpload,
  LuDatabase,
  LuFolderOpen,
  LuLoaderCircle,
  LuPackage,
  LuRotateCcw,
  LuTriangleAlert,
  LuX,
} from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import {
  AnimatedDisclosureChevron,
  AnimatedDisclosureContent,
} from "@/components/ui/animated-disclosure";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useGroupEvents } from "@/hooks/use-group-events";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { translateBackendError } from "@/lib/backend-errors";
import { fireSprinkleConfetti } from "@/lib/confetti";
import {
  type ArchiveBatchItem,
  basenameOf,
  type ClassifiedPath,
  type ImportSourceInfo,
  mergeArchiveBatchItems,
  routeImportSources,
} from "@/lib/import-source-routing";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

interface ImportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  crossOsUnlocked?: boolean;
  subPage?: boolean;
  /** Paths dropped onto the window before/while this dialog is open. */
  initialPaths?: string[];
}

type DetectedMode = "folder" | "archive" | "installation" | null;

/**
 * Three ways to bring profile data in: an existing Camoufox profile folder
 * (e.g. one saved via the upstream Python `camoufox` library), one or more
 * `.salamander-profile` archives exported from this app (`export_profiles_archive`
 * / `import_profiles_archive` — each archive is itself a bundle of 1..N
 * profiles, possibly from another machine), or an entire old installation's
 * data directory (`import_data_directory` — profiles, proxies, VPN configs,
 * extensions, settings in one go; the manual counterpart to
 * `app_dirs::migrate_from_previous_brand`, which only covers the
 * same-machine default-location case automatically).
 *
 * Rather than making the user pick a mode up front, dropping or browsing to
 * a path classifies it via `inspect_import_source` and routes automatically
 * (see `lib/import-source-routing.ts`). Archive *files* are the only kind
 * that batches at the UI level — dropping several at once imports them all
 * in one pass — on top of each individual archive already being its own
 * multi-profile bundle.
 *
 * The old multi-browser (Chrome/Brave/Edge/Vivaldi) importer relied on
 * OS-keychain secret re-sealing that only made sense for Chromium's
 * `os_crypt`, and was deleted along with Wayfern rather than ported.
 */
export function ImportProfileDialog({
  isOpen,
  onClose,
  subPage,
  initialPaths,
}: ImportProfileDialogProps) {
  const { t } = useTranslation();

  const [mode, setMode] = useState<DetectedMode>(null);
  const [isClassifying, setIsClassifying] = useState(false);
  const [detectionError, setDetectionError] = useState<string | null>(null);

  const [archiveItems, setArchiveItems] = useState<ArchiveBatchItem[]>([]);
  const [isImportingArchive, setIsImportingArchive] = useState(false);

  const [installationPath, setInstallationPath] = useState("");
  const [isImportingInstallation, setIsImportingInstallation] = useState(false);

  const [profilePath, setProfilePath] = useState("");
  const [profileName, setProfileName] = useState("");
  const [proxyAssignment, setProxyAssignment] = useState<string>("none");
  const [vpnAssignment, setVpnAssignment] = useState<string>("none");
  const [selectedGroupId, setSelectedGroupId] = useState<string>("none");
  const [isCreatingGroup, setIsCreatingGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [showFingerprint, setShowFingerprint] = useState(false);
  const [fingerprintJson, setFingerprintJson] = useState("");
  const [isImporting, setIsImporting] = useState(false);

  const { storedProxies } = useProxyEvents();
  const { groups } = useGroupEvents();
  const { vpnConfigs } = useVpnEvents();

  const classifyPaths = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      setIsClassifying(true);
      setDetectionError(null);
      try {
        const classified: ClassifiedPath[] = await Promise.all(
          paths.map(async (path) => ({
            path,
            info: await invoke<ImportSourceInfo>("inspect_import_source", {
              path,
            }),
          })),
        );
        const route = routeImportSources(classified);
        switch (route.type) {
          case "archive_batch":
            setMode("archive");
            setArchiveItems((prev) =>
              mergeArchiveBatchItems(prev, route.paths),
            );
            break;
          case "profile_folder":
            setMode("folder");
            setProfilePath(route.path);
            setProfileName((prev) =>
              prev.trim() ? prev : basenameOf(route.path),
            );
            break;
          case "data_directory":
            setMode("installation");
            setInstallationPath(route.path);
            break;
          case "mixed_error":
            setDetectionError(t("importProfile.detectMixedError"));
            break;
          case "unknown_error":
            setDetectionError(t("importProfile.detectUnknownError"));
            break;
        }
      } catch (error) {
        console.error("Failed to classify import source:", error);
        setDetectionError(t("importProfile.detectUnknownError"));
      } finally {
        setIsClassifying(false);
      }
    },
    [t],
  );

  useEffect(() => {
    if (initialPaths && initialPaths.length > 0) {
      void classifyPaths(initialPaths);
    }
    // `initialPaths` gets a new array identity from the parent on every drop
    // (including additional drops while this dialog is already open), so
    // re-running on identity change is exactly the behavior we want.
  }, [initialPaths, classifyPaths]);

  const handleBrowseArchiveFiles = async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: true,
        title: t("importProfile.archiveBrowseTitle"),
        filters: [
          { name: "Salamander Profile", extensions: ["salamander-profile"] },
        ],
      });
      const paths = Array.isArray(selected)
        ? selected
        : selected
          ? [selected]
          : [];
      if (paths.length > 0) void classifyPaths(paths);
    } catch (error) {
      console.error("Failed to open archive file dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const handleBrowseSourceFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("importProfile.selectFolderTitle"),
      });
      if (selected && typeof selected === "string") {
        void classifyPaths([selected]);
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const handleBrowseInstallation = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("importProfile.installationBrowseTitle"),
      });
      if (selected && typeof selected === "string") {
        void classifyPaths([selected]);
      }
    } catch (error) {
      console.error("Failed to open installation folder dialog:", error);
      toast.error(t("importProfile.folderDialogFailed"));
    }
  };

  const handleCreateGroup = async () => {
    const name = newGroupName.trim();
    if (!name) return;
    try {
      const group = await invoke<{ id: string; name: string }>(
        "create_profile_group",
        { name },
      );
      setSelectedGroupId(group.id);
      setIsCreatingGroup(false);
      setNewGroupName("");
    } catch (error) {
      console.error("Failed to create group:", error);
      toast.error(translateBackendError(t, error));
    }
  };

  const resetSource = () => {
    setMode(null);
    setDetectionError(null);
    setProfilePath("");
    setProfileName("");
    setProxyAssignment("none");
    setVpnAssignment("none");
    setSelectedGroupId("none");
    setIsCreatingGroup(false);
    setNewGroupName("");
    setShowFingerprint(false);
    setFingerprintJson("");
    setArchiveItems([]);
    setInstallationPath("");
  };

  const handleClose = () => {
    resetSource();
    onClose();
  };

  const handleImport = async () => {
    const path = profilePath.trim();
    const name = profileName.trim();
    if (!path || !name) return;

    setIsImporting(true);
    try {
      await invoke("import_camoufox_profile", {
        sourcePath: path,
        newProfileName: name,
        proxyId: proxyAssignment !== "none" ? proxyAssignment : undefined,
        vpnId: vpnAssignment !== "none" ? vpnAssignment : undefined,
        groupId: selectedGroupId !== "none" ? selectedGroupId : undefined,
        fingerprintJson: fingerprintJson.trim() || undefined,
      });
      toast.success(t("importProfile.camoufoxImportSuccess", { name }));
      fireSprinkleConfetti();
      handleClose();
    } catch (error) {
      console.error("Failed to import Camoufox profile:", error);
      toast.error(translateBackendError(t, error));
    } finally {
      setIsImporting(false);
    }
  };

  // Returns the names of the profiles actually created (an archive is a
  // bundle of 1..N profiles), or null on failure/already-handled.
  const importArchiveItem = async (index: number): Promise<string[] | null> => {
    const item = archiveItems[index];
    if (!item) return null;
    if (item.status === "success") return item.resultNames ?? [];
    if (item.status === "importing") return null;
    setArchiveItems((prev) =>
      prev.map((it, i) =>
        i === index
          ? { ...it, status: "importing", errorMessage: undefined }
          : it,
      ),
    );
    try {
      const imported = await invoke<BrowserProfile[]>(
        "import_profiles_archive",
        {
          archivePath: item.path,
          archivePassword: item.password.trim() || undefined,
        },
      );
      const names = imported.map((p) => p.name);
      setArchiveItems((prev) =>
        prev.map((it, i) =>
          i === index ? { ...it, status: "success", resultNames: names } : it,
        ),
      );
      return names;
    } catch (error) {
      console.error("Failed to import profile archive:", error);
      setArchiveItems((prev) =>
        prev.map((it, i) =>
          i === index
            ? {
                ...it,
                status: "error",
                errorMessage: translateBackendError(t, error),
              }
            : it,
        ),
      );
      return null;
    }
  };

  const handleBatchImport = async () => {
    setIsImportingArchive(true);
    let successItemCount = 0;
    let totalProfilesImported = 0;
    for (let i = 0; i < archiveItems.length; i++) {
      // Sequential on purpose: each import does real disk I/O (zip extraction,
      // profile/extension recreation) and progress is shown per row, so
      // racing them would just make the per-item status flicker meaninglessly.
      const names = await importArchiveItem(i);
      if (names) {
        successItemCount += 1;
        totalProfilesImported += names.length;
      }
    }
    setIsImportingArchive(false);

    if (totalProfilesImported > 0) fireSprinkleConfetti();
    if (successItemCount === archiveItems.length) {
      toast.success(
        t("importProfile.archiveBatchSuccessAll", {
          count: totalProfilesImported,
        }),
      );
      handleClose();
    } else {
      toast.message(
        t("importProfile.archiveBatchPartial", {
          success: successItemCount,
          total: archiveItems.length,
        }),
      );
    }
  };

  const removeArchiveItem = (index: number) => {
    setArchiveItems((prev) => prev.filter((_, i) => i !== index));
  };

  const updateArchiveItemPassword = (index: number, password: string) => {
    setArchiveItems((prev) =>
      prev.map((it, i) => (i === index ? { ...it, password } : it)),
    );
  };

  const handleInstallationImport = async () => {
    const path = installationPath.trim();
    if (!path) return;

    setIsImportingInstallation(true);
    try {
      const summary = await invoke<{
        profiles: number;
        proxies: number;
        vpn_configs: number;
      }>("import_data_directory", { sourcePath: path });
      toast.success(
        t("importProfile.installationImportSuccess", {
          profiles: summary.profiles,
          proxies: summary.proxies,
          vpns: summary.vpn_configs,
        }),
      );
      fireSprinkleConfetti();
      handleClose();
    } catch (error) {
      console.error("Failed to import installation data:", error);
      toast.error(translateBackendError(t, error));
    } finally {
      setIsImportingInstallation(false);
    }
  };

  const modeIcon =
    mode === "folder" ? (
      <LuFolderOpen className="size-3.5" />
    ) : mode === "archive" ? (
      <LuPackage className="size-3.5" />
    ) : mode === "installation" ? (
      <LuDatabase className="size-3.5" />
    ) : null;
  const modeLabel =
    mode === "folder"
      ? t("importProfile.modeFolder")
      : mode === "archive"
        ? t("importProfile.modeArchive")
        : mode === "installation"
          ? t("importProfile.modeInstallation")
          : "";

  return (
    <Dialog open={isOpen} onOpenChange={handleClose} subPage={subPage}>
      <DialogContent className="flex max-h-[80vh] max-w-[min(48rem,calc(100%-4rem))] flex-col">
        {!subPage && (
          <DialogHeader className="shrink-0">
            <DialogTitle>{t("importProfile.title")}</DialogTitle>
          </DialogHeader>
        )}

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div
            className={cn("space-y-4", subPage && "mx-auto w-full max-w-3xl")}
          >
            {mode === null ? (
              <>
                <div className="flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-8 text-center">
                  <LuCloudUpload className="size-8 text-muted-foreground" />
                  <p className="text-sm font-medium">
                    {t("importProfile.dropZoneTitle")}
                  </p>
                  <p className="max-w-sm text-xs text-muted-foreground">
                    {t("importProfile.dropZoneHint")}
                  </p>
                  {isClassifying && (
                    <LuLoaderCircle className="size-4 animate-spin text-muted-foreground" />
                  )}
                </div>

                {detectionError && (
                  <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive-text">
                    <LuTriangleAlert className="mt-0.5 size-4 shrink-0" />
                    <span>{detectionError}</span>
                  </div>
                )}

                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    className="flex-1 gap-2"
                    onClick={() => void handleBrowseArchiveFiles()}
                  >
                    <LuPackage className="size-4" />
                    {t("importProfile.browseArchivesButton")}
                  </Button>
                  <Button
                    variant="outline"
                    className="flex-1 gap-2"
                    onClick={() => void handleBrowseSourceFolder()}
                  >
                    <FaFolder className="size-3.5" />
                    {t("importProfile.browseFolderButton")}
                  </Button>
                </div>
              </>
            ) : (
              <div className="flex items-center justify-between rounded-lg border border-border bg-muted/50 px-3 py-2">
                <div className="flex items-center gap-1.5 text-sm font-medium">
                  {modeIcon}
                  {modeLabel}
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5 px-2 text-xs text-muted-foreground"
                  onClick={resetSource}
                >
                  <LuRotateCcw className="size-3" />
                  {t("importProfile.changeSource")}
                </Button>
              </div>
            )}

            {mode === "folder" && (
              <>
                <h3 className="text-lg font-medium">
                  {t("importProfile.camoufoxImportTitle")}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {t("importProfile.camoufoxImportHint")}
                </p>

                <div>
                  <Label htmlFor="camoufox-profile-path" className="mb-2">
                    {t("importProfile.profileFolderPath")}
                  </Label>
                  <div className="flex gap-2">
                    <Input
                      id="camoufox-profile-path"
                      value={profilePath}
                      onChange={(e) => {
                        setProfilePath(e.target.value);
                      }}
                      placeholder={t("importProfile.profileFolderPlaceholder")}
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={() => void handleBrowseSourceFolder()}
                      title={t("importProfile.browseFolderTitle")}
                    >
                      <FaFolder className="size-4" />
                    </Button>
                  </div>
                </div>

                <div>
                  <Label htmlFor="camoufox-profile-name" className="mb-2">
                    {t("importProfile.newProfileName")}
                  </Label>
                  <Input
                    id="camoufox-profile-name"
                    value={profileName}
                    onChange={(e) => {
                      setProfileName(e.target.value);
                    }}
                  />
                </div>

                <div>
                  <Label className="mb-2">
                    {t("importProfile.proxyOptional")}
                  </Label>
                  <Select
                    value={proxyAssignment}
                    onValueChange={setProxyAssignment}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder={t("importProfile.noProxy")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">
                        {t("importProfile.noProxy")}
                      </SelectItem>
                      {storedProxies.map((proxy) => (
                        <SelectItem key={proxy.id} value={proxy.id}>
                          {proxy.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {vpnConfigs.length > 0 && (
                  <div>
                    <Label className="mb-2">
                      {t("importProfile.vpnOptional")}
                    </Label>
                    <Select
                      value={vpnAssignment}
                      onValueChange={setVpnAssignment}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t("importProfile.noVpn")} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="none">
                          {t("importProfile.noVpn")}
                        </SelectItem>
                        {vpnConfigs.map((vpn) => (
                          <SelectItem key={vpn.id} value={vpn.id}>
                            {vpn.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}

                <div>
                  <Label className="mb-2">
                    {t("importProfile.groupOptional")}
                  </Label>
                  {isCreatingGroup ? (
                    <div className="flex gap-2">
                      <Input
                        value={newGroupName}
                        onChange={(e) => setNewGroupName(e.target.value)}
                        placeholder={t("importProfile.newGroupNamePlaceholder")}
                      />
                      <Button
                        variant="outline"
                        disabled={!newGroupName.trim()}
                        onClick={() => void handleCreateGroup()}
                      >
                        {t("common.buttons.create")}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => {
                          setIsCreatingGroup(false);
                          setNewGroupName("");
                        }}
                      >
                        {t("common.buttons.cancel")}
                      </Button>
                    </div>
                  ) : (
                    <Select
                      value={selectedGroupId}
                      onValueChange={(value) => {
                        if (value === "create-new") {
                          setIsCreatingGroup(true);
                        } else {
                          setSelectedGroupId(value);
                        }
                      }}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t("importProfile.noGroup")} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="none">
                          {t("importProfile.noGroup")}
                        </SelectItem>
                        {groups.map((group) => (
                          <SelectItem key={group.id} value={group.id}>
                            {group.name}
                          </SelectItem>
                        ))}
                        <SelectItem value="create-new">
                          {t("importProfile.createNewGroup")}
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  )}
                </div>

                <div>
                  <button
                    type="button"
                    className="flex cursor-pointer items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground"
                    onClick={() => setShowFingerprint((v) => !v)}
                    aria-expanded={showFingerprint}
                  >
                    <AnimatedDisclosureChevron open={showFingerprint}>
                      <LuChevronRight className="size-3.5" />
                    </AnimatedDisclosureChevron>
                    {t("importProfile.camoufoxFingerprintToggle")}
                  </button>
                  <AnimatedDisclosureContent
                    open={showFingerprint}
                    className="mt-2 space-y-2"
                  >
                    <p className="text-xs text-muted-foreground">
                      {t("importProfile.camoufoxFingerprintHint")}
                    </p>
                    <textarea
                      className="min-h-24 w-full rounded-md border border-input bg-transparent p-2 font-mono text-xs"
                      value={fingerprintJson}
                      onChange={(e) => {
                        setFingerprintJson(e.target.value);
                      }}
                      placeholder="{}"
                    />
                  </AnimatedDisclosureContent>
                </div>

                <LoadingButton
                  isLoading={isImporting}
                  disabled={!profilePath.trim() || !profileName.trim()}
                  onClick={() => void handleImport()}
                >
                  {t("importProfile.camoufoxImportButton")}
                </LoadingButton>
              </>
            )}

            {mode === "archive" && (
              <>
                <h3 className="text-lg font-medium">
                  {archiveItems.length > 1
                    ? t("importProfile.archiveBatchTitle")
                    : t("importProfile.archiveTitle")}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {archiveItems.length > 1
                    ? t("importProfile.archiveBatchHint")
                    : t("importProfile.archiveHint")}
                </p>

                <div className="space-y-2">
                  {archiveItems.map((item, index) => {
                    const names =
                      item.status === "success"
                        ? item.resultNames
                        : item.previewNames;
                    return (
                      <div
                        key={item.path}
                        className="space-y-2 rounded-md border border-border p-3"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0 flex-1">
                            <p className="truncate text-sm font-medium">
                              {names && names.length > 0
                                ? names.join(", ")
                                : item.displayName}
                            </p>
                            <p className="truncate text-xs text-muted-foreground">
                              {item.path}
                            </p>
                          </div>
                          <div className="flex shrink-0 items-center gap-2">
                            {item.status === "importing" && (
                              <LuLoaderCircle className="size-4 animate-spin text-muted-foreground" />
                            )}
                            {item.status === "success" && (
                              <span className="text-xs text-success-text">
                                {t("importProfile.archiveItemSuccess")}
                              </span>
                            )}
                            {item.status !== "importing" &&
                              item.status !== "success" && (
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  className="size-6"
                                  onClick={() => removeArchiveItem(index)}
                                  title={t("importProfile.archiveItemRemove")}
                                >
                                  <LuX className="size-3.5" />
                                </Button>
                              )}
                          </div>
                        </div>

                        {(item.needsPassword || item.status === "error") &&
                          item.status !== "success" && (
                            <Input
                              type="password"
                              value={item.password}
                              onChange={(e) =>
                                updateArchiveItemPassword(index, e.target.value)
                              }
                              placeholder={t(
                                "importProfile.archivePasswordPlaceholder",
                              )}
                              autoComplete="new-password"
                            />
                          )}

                        {item.status === "error" && item.errorMessage && (
                          <p className="text-xs text-destructive-text">
                            {item.errorMessage}
                          </p>
                        )}
                      </div>
                    );
                  })}
                </div>

                <Button
                  variant="outline"
                  size="sm"
                  className="gap-1.5"
                  onClick={() => void handleBrowseArchiveFiles()}
                >
                  <LuPackage className="size-3.5" />
                  {t("importProfile.archiveAddMore")}
                </Button>

                <LoadingButton
                  isLoading={isImportingArchive}
                  disabled={
                    archiveItems.length === 0 ||
                    archiveItems.every((item) => item.status === "success")
                  }
                  onClick={() => void handleBatchImport()}
                >
                  {archiveItems.length > 1
                    ? t("importProfile.archiveBatchImportButton", {
                        count: archiveItems.length,
                      })
                    : t("importProfile.archiveImportButton")}
                </LoadingButton>
              </>
            )}

            {mode === "installation" && (
              <>
                <h3 className="text-lg font-medium">
                  {t("importProfile.installationTitle")}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {t("importProfile.installationHint")}
                </p>

                <div>
                  <Label htmlFor="installation-folder-path" className="mb-2">
                    {t("importProfile.installationFolderPath")}
                  </Label>
                  <div className="flex gap-2">
                    <Input
                      id="installation-folder-path"
                      value={installationPath}
                      onChange={(e) => setInstallationPath(e.target.value)}
                      placeholder={t("importProfile.installationFolderPath")}
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={() => void handleBrowseInstallation()}
                      title={t("importProfile.installationBrowseTitle")}
                    >
                      <FaFolder className="size-4" />
                    </Button>
                  </div>
                </div>

                <LoadingButton
                  isLoading={isImportingInstallation}
                  disabled={!installationPath.trim()}
                  onClick={() => void handleInstallationImport()}
                >
                  {t("importProfile.installationImportButton")}
                </LoadingButton>
              </>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
