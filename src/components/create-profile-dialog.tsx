"use client";

import { invoke } from "@tauri-apps/api/core";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuCheck, LuChevronsUpDown } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { PrelaunchSitesSelector } from "@/components/prelaunch-sites-selector";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { useBrowserDownload } from "@/hooks/use-browser-download";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { getBrowserDisplayName, getBrowserIcon } from "@/lib/browser-utils";
import { DNS_BLOCKLIST_LEVELS } from "@/lib/dns-blocklist-levels";
import { cn } from "@/lib/utils";
import type {
  BrowserReleaseTypes,
  BrowserType,
  CamoufoxConfig,
  CamoufoxOS,
  ClearcoteConfig,
  PrelaunchSite,
} from "@/types";

const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

import { RippleButton } from "./ui/ripple";

type BrowserTypeString = BrowserType;

interface CreateProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateProfile: (profileData: {
    name: string;
    browserStr: BrowserTypeString;
    version: string;
    releaseType: string;
    proxyId?: string;
    vpnId?: string;
    camoufoxConfig?: CamoufoxConfig;
    browserConfig?: ClearcoteConfig;
    groupId?: string;
    extensionGroupId?: string;
    dnsBlocklist?: string;
  }) => Promise<void>;
  selectedGroupId?: string;
  crossOsUnlocked?: boolean;
}

export function CreateProfileDialog({
  isOpen,
  onClose,
  onCreateProfile,
  selectedGroupId,
  crossOsUnlocked = false,
}: CreateProfileDialogProps) {
  const { t } = useTranslation();
  const proxyListboxIdAntiDetect = useId();
  const proxyListboxIdRegular = useId();
  const [profileName, setProfileName] = useState("");
  const [activeTab, setActiveTab] = useState("anti-detect");

  const [selectedBrowser, setSelectedBrowser] =
    useState<BrowserTypeString>("camoufox");
  const [selectedProxyId, setSelectedProxyId] = useState<string>();
  const [proxyPopoverOpen, setProxyPopoverOpen] = useState(false);
  const [dnsBlocklist, setDnsBlocklist] = useState<string>("");

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>(() => ({
    os: getCurrentOS(), // Default to current OS
  }));
  const [clearcoteConfig, setClearcoteConfig] = useState<ClearcoteConfig>({});
  const [globalPrelaunchSites, setGlobalPrelaunchSites] = useState<PrelaunchSite[]>([]);

  // Reset the form fields.
  const resetForm = () => {
    setSelectedBrowser("camoufox");
    setProfileName("");
    setSelectedProxyId(undefined);
    const defaultSites = globalPrelaunchSites.map((site) => site.url);
    setCamoufoxConfig({ preset_tabs: defaultSites });
    setClearcoteConfig({ preset_tabs: defaultSites });
  };

  const handleTabChange = (value: string) => {
    setActiveTab(value);
    resetForm();
  };

  const { storedProxies } = useProxyEvents();
  const { vpnConfigs } = useVpnEvents();
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [selectedExtensionGroupId, setSelectedExtensionGroupId] =
    useState<string>();
  const [extensionGroups, setExtensionGroups] = useState<
    { id: string; name: string; extension_ids: string[] }[]
  >([]);

  useEffect(() => {
    if (isOpen) {
      void invoke<{
        global_preset_tabs?: string[];
        global_prelaunch_sites?: PrelaunchSite[];
      }>("get_app_settings")
          .then((settings) => {
            const legacySites = (settings.global_preset_tabs ?? [])
              .filter((url) => /^https?:\/\/[^\s]+$/i.test(url))
              .map((url) => ({ name: new URL(url).hostname, url }));
            const sites = (settings.global_prelaunch_sites ?? []).filter(
              (site): site is PrelaunchSite =>
                typeof site?.name === "string" &&
                site.name.trim().length > 0 &&
                typeof site?.url === "string" &&
                /^https?:\/\/[^\s]+$/i.test(site.url),
            );
            const mergedSites = [...sites];
            for (const site of legacySites) {
              if (!mergedSites.some((item) => item.url === site.url)) mergedSites.push(site);
            }
            const urls = mergedSites.map((site) => site.url);
            setGlobalPrelaunchSites(mergedSites);
            setCamoufoxConfig((previous) => ({
              ...previous,
              preset_tabs: previous.preset_tabs ?? urls,
            }));
            setClearcoteConfig((previous) => ({
              ...previous,
              preset_tabs: previous.preset_tabs ?? urls,
            }));
          })
        .catch(() => setGlobalPrelaunchSites([]));
      void invoke<{ id: string; name: string; extension_ids: string[] }[]>(
        "list_extension_groups",
      )
        .then(setExtensionGroups)
        .catch(() => {
          setExtensionGroups([]);
        });
    }
  }, [isOpen]);
  const [releaseTypes, setReleaseTypes] = useState<BrowserReleaseTypes>();
  const [isLoadingReleaseTypes, setIsLoadingReleaseTypes] = useState(false);
  const [releaseTypesError, setReleaseTypesError] = useState<string | null>(
    null,
  );
  const loadingBrowserRef = useRef<string | null>(null);

  // Use the browser download hook
  const {
    isBrowserDownloading,
    downloadBrowser,
    loadDownloadedVersions,
    isVersionDownloaded,
    downloadedVersionsMap,
  } = useBrowserDownload();

  const checkAndDownloadGeoIPDatabase = useCallback(async () => {
    try {
      const isAvailable = await invoke<boolean>("is_geoip_database_available");
      if (!isAvailable) {
        console.log("GeoIP database not available, downloading...");
        await invoke("download_geoip_database");
        console.log("GeoIP database downloaded successfully");
      }
    } catch (error) {
      console.error("Failed to check/download GeoIP database:", error);
      // Don't show error to user as this is not critical for profile creation
    }
  }, []);

  const loadReleaseTypes = useCallback(
    async (browser: string) => {
      // Set loading state
      loadingBrowserRef.current = browser;
      setIsLoadingReleaseTypes(true);
      setReleaseTypesError(null);

      try {
        const rawReleaseTypes = await invoke<BrowserReleaseTypes>(
          "get_browser_release_types",
          { browserStr: browser },
        );

        await loadDownloadedVersions(browser);

        // Only update state if this browser is still the one we're loading
        if (loadingBrowserRef.current === browser) {
          const filtered: BrowserReleaseTypes = {};
          if (rawReleaseTypes.stable) filtered.stable = rawReleaseTypes.stable;
          setReleaseTypes(filtered);
          setReleaseTypesError(null);
        }
      } catch (error) {
        console.error(`Failed to load release types for ${browser}:`, error);

        // Fallback: still load downloaded versions and derive release type from them if possible
        try {
          const downloaded = await loadDownloadedVersions(browser);
          if (loadingBrowserRef.current === browser && downloaded.length > 0) {
            const latest = downloaded[0];
            const fallback: BrowserReleaseTypes = {};
            fallback.stable = latest;
            setReleaseTypes(fallback);
            setReleaseTypesError(null);
          } else if (loadingBrowserRef.current === browser) {
            // No downloaded versions and API failed - show error
            setReleaseTypesError(
              "Failed to fetch browser versions. Please check your internet connection and try again.",
            );
          }
        } catch (e) {
          console.error(
            `Failed to load downloaded versions for ${browser}:`,
            e,
          );
          if (loadingBrowserRef.current === browser) {
            setReleaseTypesError(
              "Failed to fetch browser versions. Please check your internet connection and try again.",
            );
          }
        }
      } finally {
        // Clear loading state only if we're still loading this browser
        if (loadingBrowserRef.current === browser) {
          loadingBrowserRef.current = null;
          setIsLoadingReleaseTypes(false);
        }
      }
    },
    [loadDownloadedVersions],
  );

  // Load data when dialog opens
  useEffect(() => {
    if (isOpen) {
      // Load downloaded versions up front so the availability gate is accurate.
      void Promise.all([
        loadDownloadedVersions("camoufox"),
        loadDownloadedVersions("clearcote"),
      ]);
      // Load release types when a browser is selected
      if (selectedBrowser) {
        void loadReleaseTypes(selectedBrowser);
      }
      // Camoufox needs the GeoIP database for fingerprint generation.
      if (selectedBrowser === "camoufox") {
        void checkAndDownloadGeoIPDatabase();
      }
    }
  }, [
    isOpen,
    loadReleaseTypes,
    loadDownloadedVersions,
    checkAndDownloadGeoIPDatabase,
    selectedBrowser,
  ]);

  // Load release types when browser selection changes
  useEffect(() => {
    if (selectedBrowser) {
      // Cancel any previous loading
      loadingBrowserRef.current = null;
      // Clear previous release types immediately to prevent showing stale data
      setReleaseTypes({});
      void loadReleaseTypes(selectedBrowser);
    }
  }, [selectedBrowser, loadReleaseTypes]);

  // Helper function to get the best available version respecting rules
  const getBestAvailableVersion = useCallback(
    (_browserType?: string) => {
      if (!releaseTypes) return null;

      if (releaseTypes.stable) {
        return { version: releaseTypes.stable, releaseType: "stable" as const };
      }
      return null;
    },
    [releaseTypes],
  );

  const getCreatableVersion = useCallback(
    (browserType?: string) => {
      const bestVersion = getBestAvailableVersion(browserType);
      if (bestVersion && isVersionDownloaded(bestVersion.version)) {
        return bestVersion;
      }
      const browserDownloaded = downloadedVersionsMap[browserType ?? ""] ?? [];
      if (browserDownloaded.length > 0) {
        const fallbackVersion = browserDownloaded[0];
        return {
          version: fallbackVersion,
          releaseType: "stable" as const,
        };
      }
      return null;
    },
    [getBestAvailableVersion, isVersionDownloaded, downloadedVersionsMap],
  );

  const handleDownload = async (browserStr: string) => {
    const bestVersion = getBestAvailableVersion(browserStr);

    if (!bestVersion) {
      console.error("No version available for download");
      return;
    }

    try {
      await downloadBrowser(browserStr, bestVersion.version);
    } catch (error) {
      console.error("Failed to download browser:", error);
    }
  };

  const handleCreate = async () => {
    if (!profileName.trim()) return;

    setIsCreating(true);

    const isVpnSelection = selectedProxyId?.startsWith("vpn-") ?? false;
    const resolvedProxyId = isVpnSelection ? undefined : selectedProxyId;
    const resolvedVpnId =
      isVpnSelection && selectedProxyId ? selectedProxyId.slice(4) : undefined;

    try {
      if (activeTab === "anti-detect") {
        // Camoufox is the only anti-detect profile.
        const bestCamoufoxVersion = getCreatableVersion("camoufox");
        if (!bestCamoufoxVersion) {
          console.error("No Camoufox version available");
          return;
        }

        // The fingerprint will be generated at launch time by the Rust backend
        const finalCamoufoxConfig = {
          ...camoufoxConfig,
          preset_tabs: camoufoxConfig.preset_tabs ?? [],
        };

        await onCreateProfile({
          name: profileName.trim(),
            browserStr: "camoufox",
          version: bestCamoufoxVersion.version,
          releaseType: bestCamoufoxVersion.releaseType,
          proxyId: resolvedProxyId,
          vpnId: resolvedVpnId,
          camoufoxConfig: finalCamoufoxConfig,
          groupId:
            selectedGroupId && selectedGroupId !== "__all__"
              ? selectedGroupId
              : undefined,
          extensionGroupId: selectedExtensionGroupId,
          dnsBlocklist: dnsBlocklist || undefined,
        });
      } else {
        // Regular browser
        if (!selectedBrowser) {
          console.error("Missing required browser selection");
          return;
        }

        // Use the latest available version for the selected browser.
        const bestVersion = getCreatableVersion(selectedBrowser);
        if (!bestVersion) {
          console.error("No version available");
          return;
        }

        await onCreateProfile({
          name: profileName.trim(),
          browserStr: selectedBrowser,
          version: bestVersion.version,
          releaseType: bestVersion.releaseType,
          proxyId: resolvedProxyId,
          vpnId: resolvedVpnId,
          browserConfig: {
            ...clearcoteConfig,
            preset_tabs: clearcoteConfig.preset_tabs ?? [],
          },
          groupId:
            selectedGroupId && selectedGroupId !== "__all__"
              ? selectedGroupId
              : undefined,
          dnsBlocklist: dnsBlocklist || undefined,
        });
      }

      handleClose();
    } catch (error) {
      console.error("Failed to create profile:", error);
    } finally {
      setIsCreating(false);
    }
  };

  const handleClose = () => {
    // Cancel any ongoing loading
    loadingBrowserRef.current = null;

    // Reset all states.
    setProfileName("");
    setActiveTab("anti-detect");
    setSelectedBrowser("camoufox");
    setSelectedProxyId(undefined);
    setReleaseTypes({});
    setIsLoadingReleaseTypes(false);
    setReleaseTypesError(null);
    setCamoufoxConfig({
      os: getCurrentOS(), // Reset to current OS
    });
    setClearcoteConfig({});
    onClose();
  };

  const updateCamoufoxConfig = (
    key: keyof CamoufoxConfig,
    value: unknown,
  ) => {
    setCamoufoxConfig((prev) => ({ ...prev, [key]: value }));
  };

  // Check if browser version is downloaded and available
  const isBrowserVersionAvailable = useCallback(
    (browserStr: string) => {
      const bestVersion = getBestAvailableVersion(browserStr);
      return bestVersion && isVersionDownloaded(bestVersion.version);
    },
    [isVersionDownloaded, getBestAvailableVersion],
  );

  // Check if browser is currently downloading
  const isBrowserCurrentlyDownloading = useCallback(
    (browserStr: string) => {
      return isBrowserDownloading(browserStr);
    },
    [isBrowserDownloading],
  );

  const isCreateDisabled = useMemo(() => {
    if (!profileName.trim()) return true;
    if (!selectedBrowser) return true;
    if (isBrowserCurrentlyDownloading(selectedBrowser)) return true;
    if (!getCreatableVersion(selectedBrowser)) return true;

    return false;
  }, [
    profileName,
    selectedBrowser,
    isBrowserCurrentlyDownloading,
    getCreatableVersion,
  ]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="flex max-h-[90vh] max-w-[min(48rem,calc(100%-4rem))] flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>
            {t("createProfile.configureTitle", {
            browser: getBrowserDisplayName(selectedBrowser),
            })}
          </DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={handleTabChange}
          className="flex min-h-0 w-full flex-1 flex-col"
        >
          <div className="grid grid-cols-2 gap-2 border-b pb-4">
            {(["camoufox", "clearcote"] as const).map((browser) => {
              const Icon = getBrowserIcon(browser);
              const selected = selectedBrowser === browser;
              return (
                <Button
                  key={browser}
                  type="button"
                  variant={selected ? "secondary" : "outline"}
                  className="h-auto justify-start gap-2 py-3"
                  onClick={() => {
                    setSelectedBrowser(browser);
                    setActiveTab(browser === "camoufox" ? "anti-detect" : "regular");
                  }}
                >
                  <Icon className="size-4" />
                  <span>{getBrowserDisplayName(browser)}</span>
                </Button>
              );
            })}
           </div>

           {selectedBrowser === "clearcote" && (
            <div className="mt-4 space-y-4 rounded-lg border bg-muted/20 p-4">
              <div>
                <h3 className="font-medium">
                  {t("createProfile.engineSettings.title")}
                </h3>
                <p className="mt-1 text-sm text-muted-foreground">
                  {t("createProfile.engineSettings.description")}
                </p>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="clearcote-fingerprint-platform">
                    {t("createProfile.engineSettings.platform")}
                  </Label>
                  <Select
                    value={clearcoteConfig.fingerprint_platform ?? "auto"}
                    onValueChange={(value) =>
                      setClearcoteConfig((current) => ({
                        ...current,
                        fingerprint_platform:
                          value === "auto" ? undefined : (value as CamoufoxOS),
                      }))
                    }
                  >
                    <SelectTrigger id="clearcote-fingerprint-platform">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">
                        {t("createProfile.engineSettings.auto")}
                      </SelectItem>
                      <SelectItem value="windows">
                        {t("createProfile.os.windows")}
                      </SelectItem>
                      <SelectItem value="macos">
                        {t("createProfile.os.macos")}
                      </SelectItem>
                      <SelectItem value="linux">
                        {t("createProfile.os.linux")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="clearcote-fingerprint-seed">
                    {t("createProfile.engineSettings.seed")}
                  </Label>
                  <Input
                    id="clearcote-fingerprint-seed"
                    value={clearcoteConfig.fingerprint_seed ?? ""}
                    onChange={(event) =>
                      setClearcoteConfig((current) => ({
                        ...current,
                        fingerprint_seed: event.target.value || undefined,
                      }))
                    }
                    placeholder={t(
                      "createProfile.engineSettings.seedPlaceholder",
                    )}
                  />
                  <p className="text-xs text-muted-foreground">
                    {t("createProfile.engineSettings.seedHint")}
                  </p>
                </div>
              </div>
            </div>
          )}

          <ScrollArea className="flex-1 overflow-y-auto">
            <div className="flex w-full flex-col items-center justify-center">
              <div className="w-full space-y-6 py-4">
                    <TabsContent value="anti-detect" className="mt-0">
                      {/* Anti-Detect Configuration */}
                      <div className="space-y-6">
                        {/* Profile Name */}
                        <div className="space-y-2">
                          <Label htmlFor="profile-name">
                            {t("createProfile.profileName")}
                          </Label>
                          <Input
                            id="profile-name"
                            value={profileName}
                            onChange={(e) => {
                              setProfileName(e.target.value);
                            }}
                            onKeyDown={(e) => {
                              if (
                                e.key === "Enter" &&
                                !isCreateDisabled &&
                                !isCreating
                              ) {
                                void handleCreate();
                              }
                            }}
                            placeholder={t(
                              "createProfile.profileNamePlaceholder",
                            )}
                          />
                        </div>

                        <div className="space-y-6">
                          {/* Camoufox Download Status */}
                          {isLoadingReleaseTypes && (
                            <div className="flex items-center gap-3 rounded-md border p-3">
                              <div className="size-4 animate-spin rounded-full border-2 border-muted/40 border-t-primary" />
                              <p className="text-sm text-muted-foreground">
                                {t("createProfile.version.fetching")}
                              </p>
                            </div>
                          )}
                          {!isLoadingReleaseTypes && releaseTypesError && (
                            <div className="flex items-center gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3">
                              <p className="flex-1 text-sm text-destructive-text">
                                {releaseTypesError}
                              </p>
                              <RippleButton
                                onClick={() =>
                                  selectedBrowser &&
                                  loadReleaseTypes(selectedBrowser)
                                }
                                size="sm"
                                variant="outline"
                              >
                                {t("common.buttons.retry")}
                              </RippleButton>
                            </div>
                          )}
                          {!isLoadingReleaseTypes &&
                            !releaseTypesError &&
                            !getBestAvailableVersion("camoufox") && (
                              <div className="flex items-center gap-3 rounded-md border border-warning/50 bg-warning/10 p-3">
                                <p className="text-sm text-warning-text">
                                  {t("createProfile.platformUnavailable", {
                                    browser: "Firefox",
                                  })}
                                </p>
                              </div>
                            )}
                          {!isLoadingReleaseTypes &&
                            !releaseTypesError &&
                            !isBrowserCurrentlyDownloading("camoufox") &&
                            !getCreatableVersion("camoufox") &&
                            getBestAvailableVersion("camoufox") && (
                              <div className="flex items-center gap-3 rounded-md border p-3">
                                <p className="text-sm text-muted-foreground">
                                  {t("createProfile.version.needsDownload", {
                                    browser: "Firefox",
                                    version:
                                      getBestAvailableVersion("camoufox")
                                        ?.version,
                                  })}
                                </p>
                                <LoadingButton
                                  onClick={() => {
                                    void handleDownload("camoufox");
                                  }}
                                  isLoading={isBrowserCurrentlyDownloading(
                                    "camoufox",
                                  )}
                                  size="sm"
                                  disabled={isBrowserCurrentlyDownloading(
                                    "camoufox",
                                  )}
                                >
                                  {isBrowserCurrentlyDownloading("camoufox")
                                    ? t("common.buttons.downloading")
                                    : t("common.buttons.download")}
                                </LoadingButton>
                              </div>
                            )}
                          {!isLoadingReleaseTypes &&
                            !releaseTypesError &&
                            !isBrowserCurrentlyDownloading("camoufox") &&
                            getCreatableVersion("camoufox") && (
                              <div className="rounded-md border p-3 text-sm text-muted-foreground">
                                ✓{" "}
                                {t("createProfile.version.available", {
                                  browser: "Firefox",
                                  version:
                                    getCreatableVersion("camoufox")?.version,
                                })}
                              </div>
                            )}
                          {!isLoadingReleaseTypes &&
                            !releaseTypesError &&
                            !isBrowserCurrentlyDownloading("camoufox") &&
                            getCreatableVersion("camoufox") &&
                            !isBrowserVersionAvailable("camoufox") &&
                            getBestAvailableVersion("camoufox") && (
                              <div className="flex items-center gap-3 rounded-md border p-3">
                                <p className="flex-1 text-sm text-muted-foreground">
                                  {t(
                                    "createProfile.version.upgradeAvailable",
                                    {
                                      browser: "Firefox",
                                      version:
                                        getBestAvailableVersion("camoufox")
                                          ?.version,
                                    },
                                  )}
                                </p>
                                <LoadingButton
                                  onClick={() => {
                                    void handleDownload("camoufox");
                                  }}
                                  isLoading={isBrowserCurrentlyDownloading(
                                    "camoufox",
                                  )}
                                  size="sm"
                                  variant="outline"
                                  disabled={isBrowserCurrentlyDownloading(
                                    "camoufox",
                                  )}
                                >
                                  {isBrowserCurrentlyDownloading("camoufox")
                                    ? t("common.buttons.downloading")
                                    : t("common.buttons.download")}
                                </LoadingButton>
                              </div>
                            )}
                          {isBrowserCurrentlyDownloading("camoufox") && (
                            <div className="rounded-md border p-3 text-sm text-muted-foreground">
                              {t("createProfile.version.downloading", {
                                browser: "Firefox",
                                version:
                                  getBestAvailableVersion("camoufox")?.version,
                              })}
                            </div>
                          )}

                          {/* Camoufox fingerprint options. The fingerprint itself is
                              auto-generated at launch time by the Rust backend — there
                              is no field-by-field editor (Camoufox has no equivalent to
                              Wayfern's device-synthesis fingerprint model). */}
                          <div className="space-y-4 rounded-md border p-3">
                            <div className="space-y-2">
                              <Label>{t("createProfile.os.label")}</Label>
                              <Select
                                value={camoufoxConfig.os ?? getCurrentOS()}
                                onValueChange={(value) => {
                                  updateCamoufoxConfig("os", value as CamoufoxOS);
                                }}
                                disabled={!crossOsUnlocked}
                              >
                                <SelectTrigger>
                                  <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                  <SelectItem value="windows">
                                    {t("createProfile.os.windows")}
                                  </SelectItem>
                                  <SelectItem value="macos">
                                    {t("createProfile.os.macos")}
                                  </SelectItem>
                                  <SelectItem value="linux">
                                    {t("createProfile.os.linux")}
                                  </SelectItem>
                                </SelectContent>
                              </Select>
                            </div>

                            <div className="grid grid-cols-2 gap-3">
                              {([
                                [
                                  "screen_min_width",
                                  "createProfile.screen.minWidth",
                                ],
                                [
                                  "screen_max_width",
                                  "createProfile.screen.maxWidth",
                                ],
                                [
                                  "screen_min_height",
                                  "createProfile.screen.minHeight",
                                ],
                                [
                                  "screen_max_height",
                                  "createProfile.screen.maxHeight",
                                ],
                              ] as const).map(([key, label]) => (
                                <div className="space-y-2" key={key}>
                                  <Label htmlFor={`camoufox-${key}`}>
                                    {t(label)}
                                  </Label>
                                  <Input
                                    id={`camoufox-${key}`}
                                    type="number"
                                    min={1}
                                    value={camoufoxConfig[key] ?? ""}
                                    onChange={(event) =>
                                      updateCamoufoxConfig(
                                        key,
                                        event.target.value
                                          ? Number(event.target.value)
                                          : undefined,
                                      )
                                    }
                                    placeholder={t(
                                      "createProfile.screen.autoPlaceholder",
                                    )}
                                  />
                                </div>
                              ))}
                            </div>

                            <div className="flex items-center justify-between gap-4">
                              <div>
                                <Label htmlFor="camoufox-randomize-fingerprint">
                                  {t("createProfile.randomizeFingerprint.label")}
                                </Label>
                                <p className="mt-1 text-xs text-muted-foreground">
                                  {t("createProfile.randomizeFingerprint.description")}
                                </p>
                              </div>
                              <Checkbox
                                id="camoufox-randomize-fingerprint"
                                checked={
                                  camoufoxConfig.randomize_fingerprint_on_launch ??
                                  false
                                }
                                onCheckedChange={(checked) =>
                                  updateCamoufoxConfig(
                                    "randomize_fingerprint_on_launch",
                                    checked === true,
                                  )
                                }
                              />
                            </div>

                            <div className="flex items-center justify-between">
                              <Label htmlFor="camoufox-block-images">
                                {t("createProfile.blockImages")}
                              </Label>
                              <Checkbox
                                id="camoufox-block-images"
                                checked={camoufoxConfig.block_images ?? false}
                                onCheckedChange={(checked) => {
                                  updateCamoufoxConfig(
                                    "block_images",
                                    checked === true,
                                  );
                                }}
                              />
                            </div>

                            <div className="flex items-center justify-between">
                              <Label htmlFor="camoufox-block-webrtc">
                                {t("createProfile.blockWebrtc")}
                              </Label>
                              <Checkbox
                                id="camoufox-block-webrtc"
                                checked={camoufoxConfig.block_webrtc ?? false}
                                onCheckedChange={(checked) => {
                                  updateCamoufoxConfig(
                                    "block_webrtc",
                                    checked === true,
                                  );
                                }}
                              />
                            </div>

                            <div className="flex items-center justify-between">
                              <Label htmlFor="camoufox-block-webgl">
                                {t("createProfile.blockWebgl")}
                              </Label>
                              <Checkbox
                                id="camoufox-block-webgl"
                                checked={camoufoxConfig.block_webgl ?? false}
                                onCheckedChange={(checked) => {
                                  updateCamoufoxConfig(
                                    "block_webgl",
                                    checked === true,
                                  );
                                }}
                              />
                            </div>

                            <div className="flex items-center justify-between">
                              <Label htmlFor="camoufox-geoip">
                                {t("createProfile.geoip")}
                              </Label>
                              <Checkbox
                                id="camoufox-geoip"
                                checked={camoufoxConfig.geoip !== false}
                                onCheckedChange={(checked) => {
                                  updateCamoufoxConfig(
                                    "geoip",
                                    checked === true,
                                  );
                                }}
                              />
                            </div>

                            <div className="space-y-2">
                              <Label>{t("createProfile.presetTabs.label")}</Label>
                              <PrelaunchSitesSelector
                                idPrefix="camoufox-preset"
                                t={t}
                                customSites={globalPrelaunchSites}
                                selectedUrls={camoufoxConfig.preset_tabs ?? []}
                                onChange={(urls) => updateCamoufoxConfig("preset_tabs", urls)}
                              />
                            </div>
                          </div>
                        </div>

                        {/* Proxy / VPN Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex items-center justify-between">
                            <Label>{t("createProfile.proxy.title")}</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => {
                                setShowProxyForm(true);
                              }}
                              className="h-7 px-2 text-xs"
                            >
                              <GoPlus className="mr-1 size-3" />{" "}
                              {t("createProfile.proxy.addProxy")}
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 || vpnConfigs.length > 0 ? (
                            <Popover
                              open={proxyPopoverOpen}
                              onOpenChange={setProxyPopoverOpen}
                            >
                              <PopoverTrigger asChild>
                                <Button
                                  variant="outline"
                                  role="combobox"
                                  aria-expanded={proxyPopoverOpen}
                                  aria-controls={proxyListboxIdAntiDetect}
                                  className="w-full justify-between font-normal"
                                >
                                  {(() => {
                                    if (!selectedProxyId)
                                      return t("createProfile.proxy.noProxy");
                                    if (selectedProxyId.startsWith("vpn-")) {
                                      const vpn = vpnConfigs.find(
                                        (v) =>
                                          v.id === selectedProxyId.slice(4),
                                      );
                                      return vpn
                                         ? `WG — ${vpn.name}`
                                        : t("createProfile.proxy.noProxy");
                                    }
                                    const proxy = storedProxies.find(
                                      (p) => p.id === selectedProxyId,
                                    );
                                    return (
                                      proxy?.name ??
                                      t("createProfile.proxy.noProxy")
                                    );
                                  })()}
                                  <LuChevronsUpDown className="ml-2 size-4 shrink-0 opacity-50" />
                                </Button>
                              </PopoverTrigger>
                              <PopoverContent
                                id={proxyListboxIdAntiDetect}
                                className="w-[240px] p-0"
                                sideOffset={8}
                              >
                                <Command>
                                  <CommandInput
                                    placeholder={t(
                                      "createProfile.proxy.search",
                                    )}
                                  />
                                  <CommandList>
                                    <CommandEmpty>
                                      {t("createProfile.proxy.notFound")}
                                    </CommandEmpty>
                                    <CommandGroup>
                                      <CommandItem
                                        value="__none__"
                                        onSelect={() => {
                                          setSelectedProxyId(undefined);
                                          setProxyPopoverOpen(false);
                                        }}
                                      >
                                        <LuCheck
                                          className={cn(
                                            "mr-2 size-4",
                                            !selectedProxyId
                                              ? "opacity-100"
                                              : "opacity-0",
                                          )}
                                        />
                                        {t("common.labels.none")}
                                      </CommandItem>
                                      {storedProxies.map((proxy) => (
                                        <CommandItem
                                          key={proxy.id}
                                          value={proxy.name}
                                          onSelect={() => {
                                            setSelectedProxyId(proxy.id);
                                            setProxyPopoverOpen(false);
                                          }}
                                        >
                                          <LuCheck
                                            className={cn(
                                              "mr-2 size-4",
                                              selectedProxyId === proxy.id
                                                ? "opacity-100"
                                                : "opacity-0",
                                            )}
                                          />
                                          {proxy.name}
                                        </CommandItem>
                                      ))}
                                    </CommandGroup>
                                    {vpnConfigs.length > 0 && (
                                      <CommandGroup heading="VPNs">
                                        {vpnConfigs.map((vpn) => (
                                          <CommandItem
                                            key={vpn.id}
                                            value={`vpn-${vpn.name}`}
                                            onSelect={() => {
                                              setSelectedProxyId(
                                                `vpn-${vpn.id}`,
                                              );
                                              setProxyPopoverOpen(false);
                                            }}
                                          >
                                            <LuCheck
                                              className={cn(
                                                "mr-2 size-4",
                                                selectedProxyId ===
                                                  `vpn-${vpn.id}`
                                                  ? "opacity-100"
                                                  : "opacity-0",
                                              )}
                                            />
                                            <Badge
                                              variant="outline"
                                              className="mr-1 px-1 py-0 text-[10px] leading-tight"
                                            >
                                               WG
                                            </Badge>
                                            {vpn.name}
                                          </CommandItem>
                                        ))}
                                      </CommandGroup>
                                    )}
                                  </CommandList>
                                </Command>
                              </PopoverContent>
                            </Popover>
                          ) : (
                            <div className="flex items-center gap-3 rounded-md border p-3 text-sm text-muted-foreground">
                              {t("createProfile.proxy.noProxiesAvailable")}
                            </div>
                          )}
                        </div>

                        {/* DNS Blocklist */}
                        <div className="space-y-2">
                          <Label>{t("dnsBlocklist.title")}</Label>
                          <Select
                            value={dnsBlocklist || "none"}
                            onValueChange={(val) => {
                              setDnsBlocklist(val === "none" ? "" : val);
                            }}
                          >
                            <SelectTrigger>
                              <SelectValue
                                placeholder={t("dnsBlocklist.none")}
                              />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="none">
                                {t("dnsBlocklist.none")}
                              </SelectItem>
                              {DNS_BLOCKLIST_LEVELS.map((level) => (
                                <SelectItem
                                  key={level.value}
                                  value={level.value}
                                >
                                  {t(level.labelKey)}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>

                        {/* Extension Group */}
                        {extensionGroups.length > 0 && (
                          <div className="space-y-2">
                            <Label>{t("extensions.extensionGroup")}</Label>
                            <Select
                              value={selectedExtensionGroupId ?? "none"}
                              onValueChange={(val) => {
                                setSelectedExtensionGroupId(
                                  val === "none" ? undefined : val,
                                );
                              }}
                            >
                              <SelectTrigger>
                                <SelectValue
                                  placeholder={t("profileInfo.values.none")}
                                />
                              </SelectTrigger>
                              <SelectContent>
                                <SelectItem value="none">
                                  {t("profileInfo.values.none")}
                                </SelectItem>
                                {extensionGroups.map((g) => (
                                  <SelectItem key={g.id} value={g.id}>
                                    {g.name} ({g.extension_ids.length})
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          </div>
                        )}
                      </div>
                    </TabsContent>

                    <TabsContent value="regular" className="mt-0">
                      {/* Regular Browser Configuration */}
                      <div className="space-y-6">
                        {/* Profile Name */}
                        <div className="space-y-2">
                          <Label htmlFor="profile-name">
                            {t("createProfile.profileName")}
                          </Label>
                          <Input
                            id="profile-name"
                            value={profileName}
                            onChange={(e) => {
                              setProfileName(e.target.value);
                            }}
                            onKeyDown={(e) => {
                              if (
                                e.key === "Enter" &&
                                !isCreateDisabled &&
                                !isCreating
                              ) {
                                void handleCreate();
                              }
                            }}
                            placeholder={t(
                              "createProfile.profileNamePlaceholder",
                            )}
                          />
                        </div>

                        <div className="space-y-3 rounded-lg border bg-muted/30 p-4">
                          <Label className="font-medium">
                            {t("createProfile.presetTabs.label")}
                          </Label>
                           <PrelaunchSitesSelector
                             idPrefix="clearcote-preset"
                             t={t}
                             customSites={globalPrelaunchSites}
                             selectedUrls={clearcoteConfig.preset_tabs ?? []}
                             onChange={(urls) =>
                               setClearcoteConfig((prev) => ({
                                 ...prev,
                                 preset_tabs: urls,
                               }))
                             }
                           />
                        </div>

                        {/* Regular Browser Configuration */}
                        <div className="space-y-4">
                          {selectedBrowser && (
                            <div className="space-y-3">
                              {isLoadingReleaseTypes && (
                                <div className="flex items-center gap-3">
                                  <div className="size-4 animate-spin rounded-full border-2 border-muted/40 border-t-primary" />
                                  <p className="text-sm text-muted-foreground">
                                    {t("createProfile.version.fetching")}
                                  </p>
                                </div>
                              )}
                              {!isLoadingReleaseTypes && releaseTypesError && (
                                <div className="flex items-center gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3">
                                  <p className="flex-1 text-sm text-destructive-text">
                                    {releaseTypesError}
                                  </p>
                                  <RippleButton
                                    onClick={() =>
                                      selectedBrowser &&
                                      loadReleaseTypes(selectedBrowser)
                                    }
                                    size="sm"
                                    variant="outline"
                                  >
                                    {t("common.buttons.retry")}
                                  </RippleButton>
                                </div>
                              )}
                              {!isLoadingReleaseTypes &&
                                !releaseTypesError &&
                                !isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                ) &&
                                !getCreatableVersion(selectedBrowser) &&
                                getBestAvailableVersion(selectedBrowser) && (
                                  <div className="flex items-center gap-3">
                                    <p className="text-sm text-muted-foreground">
                                      {t(
                                        "createProfile.version.latestNeedsDownload",
                                        {
                                          version:
                                            getBestAvailableVersion(
                                              selectedBrowser,
                                            )?.version,
                                        },
                                      )}
                                    </p>
                                    <LoadingButton
                                      onClick={() => {
                                        void handleDownload(selectedBrowser);
                                      }}
                                      isLoading={isBrowserCurrentlyDownloading(
                                        selectedBrowser,
                                      )}
                                      className="ml-auto"
                                      size="sm"
                                      disabled={isBrowserCurrentlyDownloading(
                                        selectedBrowser,
                                      )}
                                    >
                                      {t("common.buttons.download")}
                                    </LoadingButton>
                                  </div>
                                )}
                              {!isLoadingReleaseTypes &&
                                !releaseTypesError &&
                                !isBrowserCurrentlyDownloading(
                                  selectedBrowser,
                                ) &&
                                getCreatableVersion(selectedBrowser) && (
                                  <div className="text-sm text-muted-foreground">
                                    ✓{" "}
                                    {t(
                                      "createProfile.version.latestAvailable",
                                      {
                                        version:
                                          getCreatableVersion(selectedBrowser)
                                            ?.version,
                                      },
                                    )}
                                  </div>
                                )}
                              {isBrowserCurrentlyDownloading(
                                selectedBrowser,
                              ) && (
                                <div className="text-sm text-muted-foreground">
                                  {t(
                                    "createProfile.version.latestDownloading",
                                    {
                                      version:
                                        getBestAvailableVersion(selectedBrowser)
                                          ?.version,
                                    },
                                  )}
                                </div>
                              )}
                            </div>
                          )}
                        </div>

                        {/* Proxy / VPN Selection - Always visible */}
                        <div className="space-y-3">
                          <div className="flex items-center justify-between">
                            <Label>{t("createProfile.proxy.title")}</Label>
                            <RippleButton
                              size="sm"
                              variant="outline"
                              onClick={() => {
                                setShowProxyForm(true);
                              }}
                              className="h-7 px-2 text-xs"
                            >
                              <GoPlus className="mr-1 size-3" />{" "}
                              {t("createProfile.proxy.addProxy")}
                            </RippleButton>
                          </div>
                          {storedProxies.length > 0 || vpnConfigs.length > 0 ? (
                            <Popover
                              open={proxyPopoverOpen}
                              onOpenChange={setProxyPopoverOpen}
                            >
                              <PopoverTrigger asChild>
                                <Button
                                  variant="outline"
                                  role="combobox"
                                  aria-expanded={proxyPopoverOpen}
                                  aria-controls={proxyListboxIdRegular}
                                  className="w-full justify-between font-normal"
                                >
                                  {(() => {
                                    if (!selectedProxyId)
                                      return t("createProfile.proxy.noProxy");
                                    if (selectedProxyId.startsWith("vpn-")) {
                                      const vpn = vpnConfigs.find(
                                        (v) =>
                                          v.id === selectedProxyId.slice(4),
                                      );
                                      return vpn
                                         ? `WG — ${vpn.name}`
                                        : t("createProfile.proxy.noProxy");
                                    }
                                    const proxy = storedProxies.find(
                                      (p) => p.id === selectedProxyId,
                                    );
                                    return (
                                      proxy?.name ??
                                      t("createProfile.proxy.noProxy")
                                    );
                                  })()}
                                  <LuChevronsUpDown className="ml-2 size-4 shrink-0 opacity-50" />
                                </Button>
                              </PopoverTrigger>
                              <PopoverContent
                                id={proxyListboxIdRegular}
                                className="w-[240px] p-0"
                                sideOffset={8}
                              >
                                <Command>
                                  <CommandInput
                                    placeholder={t(
                                      "createProfile.proxy.search",
                                    )}
                                  />
                                  <CommandList>
                                    <CommandEmpty>
                                      {t("createProfile.proxy.notFound")}
                                    </CommandEmpty>
                                    <CommandGroup>
                                      <CommandItem
                                        value="__none__"
                                        onSelect={() => {
                                          setSelectedProxyId(undefined);
                                          setProxyPopoverOpen(false);
                                        }}
                                      >
                                        <LuCheck
                                          className={cn(
                                            "mr-2 size-4",
                                            !selectedProxyId
                                              ? "opacity-100"
                                              : "opacity-0",
                                          )}
                                        />
                                        {t("common.labels.none")}
                                      </CommandItem>
                                      {storedProxies.map((proxy) => (
                                        <CommandItem
                                          key={proxy.id}
                                          value={proxy.name}
                                          onSelect={() => {
                                            setSelectedProxyId(proxy.id);
                                            setProxyPopoverOpen(false);
                                          }}
                                        >
                                          <LuCheck
                                            className={cn(
                                              "mr-2 size-4",
                                              selectedProxyId === proxy.id
                                                ? "opacity-100"
                                                : "opacity-0",
                                            )}
                                          />
                                          {proxy.name}
                                        </CommandItem>
                                      ))}
                                    </CommandGroup>
                                    {vpnConfigs.length > 0 && (
                                      <CommandGroup heading="VPNs">
                                        {vpnConfigs.map((vpn) => (
                                          <CommandItem
                                            key={vpn.id}
                                            value={`vpn-${vpn.name}`}
                                            onSelect={() => {
                                              setSelectedProxyId(
                                                `vpn-${vpn.id}`,
                                              );
                                              setProxyPopoverOpen(false);
                                            }}
                                          >
                                            <LuCheck
                                              className={cn(
                                                "mr-2 size-4",
                                                selectedProxyId ===
                                                  `vpn-${vpn.id}`
                                                  ? "opacity-100"
                                                  : "opacity-0",
                                              )}
                                            />
                                            <Badge
                                              variant="outline"
                                              className="mr-1 px-1 py-0 text-[10px] leading-tight"
                                            >
                                               WG
                                            </Badge>
                                            {vpn.name}
                                          </CommandItem>
                                        ))}
                                      </CommandGroup>
                                    )}
                                  </CommandList>
                                </Command>
                              </PopoverContent>
                            </Popover>
                          ) : (
                            <div className="flex items-center gap-3 rounded-md border p-3 text-sm text-muted-foreground">
                              {t("createProfile.proxy.noProxiesAvailable")}
                            </div>
                          )}
                        </div>
                      </div>
                    </TabsContent>
              </div>
            </div>
          </ScrollArea>
        </Tabs>

        <DialogFooter className="shrink-0 border-t pt-4">
          <RippleButton variant="outline" onClick={handleClose}>
            {t("common.buttons.close")}
          </RippleButton>
          <LoadingButton
            onClick={handleCreate}
            isLoading={isCreating}
            disabled={isCreateDisabled}
          >
            {t("common.buttons.create")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={() => {
          setShowProxyForm(false);
        }}
      />
    </Dialog>
  );
}
