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
import { LuCheck, LuChevronsUpDown, LuLoaderCircle } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
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
import { getBrowserIcon } from "@/lib/browser-utils";
import { DNS_BLOCKLIST_LEVELS } from "@/lib/dns-blocklist-levels";
import { PRESET_TAB_OPTIONS } from "@/lib/preset-tabs";
import { cn } from "@/lib/utils";
import type { BrowserReleaseTypes, CamoufoxConfig, CamoufoxOS } from "@/types";

const getCurrentOS = (): CamoufoxOS => {
  if (typeof navigator === "undefined") return "linux";
  const platform = navigator.platform.toLowerCase();
  if (platform.includes("win")) return "windows";
  if (platform.includes("mac")) return "macos";
  return "linux";
};

import { RippleButton } from "./ui/ripple";

type BrowserTypeString = "camoufox";

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
    groupId?: string;
    extensionGroupId?: string;
    ephemeral?: boolean;
    dnsBlocklist?: string;
    launchHook?: string;
    password?: string;
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

  // Camoufox is the only creatable browser, so there is no browser-selection
  // screen — the dialog opens straight into the config step.
  const [selectedBrowser, setSelectedBrowser] =
    useState<BrowserTypeString>("camoufox");
  const [selectedProxyId, setSelectedProxyId] = useState<string>();
  const [proxyPopoverOpen, setProxyPopoverOpen] = useState(false);
  const [dnsBlocklist, setDnsBlocklist] = useState<string>("");
  const [launchHook, setLaunchHook] = useState("");

  // Camoufox anti-detect states
  const [camoufoxConfig, setCamoufoxConfig] = useState<CamoufoxConfig>(() => ({
    os: getCurrentOS(), // Default to current OS
  }));

  // Reset the form fields.
  const resetForm = () => {
    setSelectedBrowser("camoufox");
    setProfileName("");
    setSelectedProxyId(undefined);
    setLaunchHook("");
  };

  const handleTabChange = (value: string) => {
    setActiveTab(value);
    resetForm();
  };

  const { storedProxies } = useProxyEvents();
  const { vpnConfigs } = useVpnEvents();
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [ephemeral, setEphemeral] = useState(false);
  const [enablePassword, setEnablePassword] = useState(false);
  const [password, setPassword] = useState("");
  const [passwordConfirm, setPasswordConfirm] = useState("");
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const PASSWORD_MIN_LEN = 8;
  const [selectedExtensionGroupId, setSelectedExtensionGroupId] =
    useState<string>();
  const [extensionGroups, setExtensionGroups] = useState<
    { id: string; name: string; extension_ids: string[] }[]
  >([]);

  useEffect(() => {
    if (isOpen) {
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
      // Load downloaded Camoufox versions up front so the availability gate is accurate.
      void loadDownloadedVersions("camoufox");
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

    if (enablePassword && !ephemeral) {
      if (password.length < PASSWORD_MIN_LEN) {
        setPasswordError(
          t("profilePassword.errors.tooShort", { min: PASSWORD_MIN_LEN }),
        );
        return;
      }
      if (password !== passwordConfirm) {
        setPasswordError(t("profilePassword.errors.mismatch"));
        return;
      }
    }
    setPasswordError(null);

    setIsCreating(true);

    const isVpnSelection = selectedProxyId?.startsWith("vpn-") ?? false;
    const resolvedProxyId = isVpnSelection ? undefined : selectedProxyId;
    const resolvedVpnId =
      isVpnSelection && selectedProxyId ? selectedProxyId.slice(4) : undefined;

    const passwordToSet =
      enablePassword && !ephemeral && password.length >= PASSWORD_MIN_LEN
        ? password
        : undefined;
    try {
      if (activeTab === "anti-detect") {
        // Only Camoufox anti-detect profiles are created.
        const bestCamoufoxVersion = getCreatableVersion("camoufox");
        if (!bestCamoufoxVersion) {
          console.error("No Camoufox version available");
          return;
        }

        // The fingerprint will be generated at launch time by the Rust backend
        const finalCamoufoxConfig = { ...camoufoxConfig };

        await onCreateProfile({
          name: profileName.trim(),
          browserStr: "camoufox" as BrowserTypeString,
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
          ephemeral,
          dnsBlocklist: dnsBlocklist || undefined,
          launchHook: launchHook.trim() || undefined,
          password: passwordToSet,
        });
      } else {
        // Regular browser
        if (!selectedBrowser) {
          console.error("Missing required browser selection");
          return;
        }

        // Use the latest available Camoufox version
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
          proxyId: selectedProxyId,
          groupId:
            selectedGroupId && selectedGroupId !== "__all__"
              ? selectedGroupId
              : undefined,
          dnsBlocklist: dnsBlocklist || undefined,
          launchHook: launchHook.trim() || undefined,
          password: passwordToSet,
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
    setLaunchHook("");
    setReleaseTypes({});
    setIsLoadingReleaseTypes(false);
    setReleaseTypesError(null);
    setCamoufoxConfig({
      os: getCurrentOS(), // Reset to current OS
    });
    setEphemeral(false);
    setEnablePassword(false);
    setPassword("");
    setPasswordConfirm("");
    setPasswordError(null);
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
              browser: t("createProfile.chromiumLabel"),
            })}
          </DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={handleTabChange}
          className="flex min-h-0 w-full flex-1 flex-col"
        >
          {/* Tab list hidden - only anti-detect browsers are supported */}

          <ScrollArea className="flex-1 overflow-y-auto">
            <div className="flex w-full flex-col items-center justify-center">
              <div className="w-full space-y-6 py-4">
                <>
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

                        {/* Ephemeral Option */}
                        <div className="space-y-3 rounded-lg border bg-muted/30 p-4">
                          <div className="flex items-center gap-x-2">
                            <Checkbox
                              id="ephemeral"
                              checked={ephemeral}
                              onCheckedChange={(checked) => {
                                setEphemeral(checked === true);
                              }}
                            />
                            <Label htmlFor="ephemeral" className="font-medium">
                              {t("profiles.ephemeral")}
                            </Label>
                          </div>
                          <p className="ml-6 text-sm text-muted-foreground">
                            {t("profiles.ephemeralDescription")}
                          </p>
                        </div>

                        {/* Password Option */}
                        {!ephemeral && (
                          <div className="space-y-3 rounded-lg border bg-muted/30 p-4">
                            <div className="flex items-center gap-x-2">
                              <Checkbox
                                id="enable-password"
                                checked={enablePassword}
                                onCheckedChange={(checked) => {
                                  setEnablePassword(checked === true);
                                  if (checked !== true) {
                                    setPassword("");
                                    setPasswordConfirm("");
                                    setPasswordError(null);
                                  }
                                }}
                              />
                              <Label
                                htmlFor="enable-password"
                                className="font-medium"
                              >
                                {t("createProfile.passwordProtect.label")}
                              </Label>
                            </div>
                            <p className="ml-6 text-sm text-muted-foreground">
                              {t("createProfile.passwordProtect.description")}
                            </p>
                            {enablePassword && (
                              <div className="ml-6 space-y-2">
                                <Input
                                  type="password"
                                  value={password}
                                  onChange={(e) => {
                                    setPassword(e.target.value);
                                    setPasswordError(null);
                                  }}
                                  placeholder={t(
                                    "profilePassword.fields.newPassword",
                                  )}
                                  autoComplete="new-password"
                                />
                                <Input
                                  type="password"
                                  value={passwordConfirm}
                                  onChange={(e) => {
                                    setPasswordConfirm(e.target.value);
                                    setPasswordError(null);
                                  }}
                                  placeholder={t(
                                    "profilePassword.fields.confirm",
                                  )}
                                  autoComplete="new-password"
                                />
                                {passwordError && (
                                  <p className="text-sm text-destructive-text">
                                    {passwordError}
                                  </p>
                                )}
                              </div>
                            )}
                          </div>
                        )}

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
                                    browser: "Camoufox",
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
                                    browser: "Camoufox",
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
                                  browser: "Camoufox",
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
                                      browser: "Camoufox",
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
                                browser: "Camoufox",
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
                              <div className="space-y-2">
                                {PRESET_TAB_OPTIONS.map((preset) => (
                                  <div
                                    key={preset.url}
                                    className="flex items-center justify-between"
                                  >
                                    <Label htmlFor={`camoufox-preset-${preset.key}`}>
                                      {t(`createProfile.presetTabs.${preset.key}`)}
                                    </Label>
                                    <Checkbox
                                      id={`camoufox-preset-${preset.key}`}
                                      checked={(camoufoxConfig.preset_tabs ?? []).includes(
                                        preset.url,
                                      )}
                                      onCheckedChange={(checked) => {
                                        const current = camoufoxConfig.preset_tabs ?? [];
                                        const next =
                                          checked === true
                                            ? [...current, preset.url]
                                            : current.filter((u) => u !== preset.url);
                                        updateCamoufoxConfig("preset_tabs", next);
                                      }}
                                    />
                                  </div>
                                ))}
                              </div>
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

                        <div className="space-y-2">
                          <Label htmlFor="launch-hook-url">
                            {t("createProfile.launchHook.label")}
                          </Label>
                          <Input
                            id="launch-hook-url"
                            value={launchHook}
                            onChange={(e) => {
                              setLaunchHook(e.target.value);
                            }}
                            placeholder={t(
                              "createProfile.launchHook.placeholder",
                            )}
                            disabled={isCreating}
                          />
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

                        <div className="space-y-2">
                          <Label htmlFor="launch-hook-url-regular">
                            {t("createProfile.launchHook.label")}
                          </Label>
                          <Input
                            id="launch-hook-url-regular"
                            value={launchHook}
                            onChange={(e) => {
                              setLaunchHook(e.target.value);
                            }}
                            placeholder={t(
                              "createProfile.launchHook.placeholder",
                            )}
                            disabled={isCreating}
                          />
                        </div>
                      </div>
                    </TabsContent>
                  </>
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
