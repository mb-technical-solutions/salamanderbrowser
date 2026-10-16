"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import Color from "color";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { FaApple, FaLinux, FaWindows } from "react-icons/fa";
import {
  LuChevronRight,
  LuClipboard,
  LuClipboardCheck,
  LuCookie,
  LuCopy,
  LuDownload,
  LuFingerprint,
  LuGlobe,
  LuGroup,
  LuPlus,
  LuPuzzle,
  LuSettings,
  LuShield,
  LuShieldCheck,
  LuTrash2,
  LuUpload,
  LuUsers,
  LuX,
} from "react-icons/lu";
import { ExportProfileDialog } from "@/components/export-profile-dialog";
import { PrelaunchSitesSelector } from "@/components/prelaunch-sites-selector";
import {
  ProfilePasswordDialog,
  type PasswordDialogMode,
} from "@/components/profile-password-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  ColorPicker,
  ColorPickerEyeDropper,
  ColorPickerFormat,
  ColorPickerHue,
  ColorPickerOutput,
  ColorPickerSelection,
} from "@/components/ui/color-picker";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
import { translateBackendError } from "@/lib/backend-errors";
import {
  getProfileIcon,
  supportsCookieBackend,
} from "@/lib/browser-utils";
import { DNS_BLOCKLIST_LEVELS } from "@/lib/dns-blocklist-levels";
import { formatRelativeTime } from "@/lib/flag-utils";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type {
  BrowserProfile,
  CamoufoxConfig,
  CamoufoxOS,
  ProfileGroup,
  PrelaunchSite,
  StoredProxy,
  VpnConfig,
} from "@/types";

interface ProfileInfoDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  onOpenTrafficDialog?: (profileId: string) => void;
  onAssignProfilesToGroup?: (profileIds: string[]) => void;
  onCopyCookiesToProfile?: (profile: BrowserProfile) => void;
  onOpenCookieManagement?: (profile: BrowserProfile) => void;
  onAssignExtensionGroup?: (profileIds: string[]) => void;
  onOpenBypassRules?: (profile: BrowserProfile) => void;
  onOpenDnsBlocklist?: (profile: BrowserProfile) => void;
  onCloneProfile?: (profile: BrowserProfile) => void;
  onDeleteProfile?: (profile: BrowserProfile) => void;
  crossOsUnlocked?: boolean;
  isRunning?: boolean;
  isDisabled?: boolean;
  isCrossOs?: boolean;
}

function _OSIcon({ os }: { os: string }) {
  switch (os) {
    case "macos":
      return <FaApple className="size-3.5" />;
    case "windows":
      return <FaWindows className="size-3.5" />;
    case "linux":
      return <FaLinux className="size-3.5" />;
    default:
      return null;
  }
}

function InfoCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border bg-muted/50 px-3 py-2.5">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-0.5 truncate text-sm">{value}</p>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * Shows the total bytes routed through Salamander's local proxy worker for this
 * profile. Only counts traffic flowing through the salamander-proxy binary — not
 * the browser's full network usage, hence the "Local" qualifier.
 */
function LocalDataTransferCard({
  profileId,
  t,
}: {
  profileId: string;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type Snapshot = {
    total_bytes_sent: number;
    total_bytes_received: number;
  };
  const [value, setValue] = React.useState<string>("—");

  React.useEffect(() => {
    let mounted = true;
    const fetchSnapshot = async () => {
      try {
        const snap = await invoke<Snapshot | null>(
          "get_profile_traffic_snapshot",
          { profileId },
        );
        if (!mounted) return;
        if (!snap) {
          setValue("0 B");
          return;
        }
        setValue(
          formatBytes(snap.total_bytes_sent + snap.total_bytes_received),
        );
      } catch {
        if (mounted) setValue("—");
      }
    };
    void fetchSnapshot();
    const interval = window.setInterval(fetchSnapshot, 5000);
    return () => {
      mounted = false;
      window.clearInterval(interval);
    };
  }, [profileId]);

  return (
    <InfoCard label={t("profileInfo.fields.localDataTransfer")} value={value} />
  );
}

// Shown only for legacy profiles that predate the feature and have no stored
// color yet (new profiles get a backend-derived one at creation/launch).
const DEFAULT_SWATCH_COLOR = "#94a3b8";

function WindowColorSwatch({ profile }: { profile: BrowserProfile }) {
  const { t } = useTranslation();
  const [color, setColor] = React.useState(profile.window_color);

  React.useEffect(() => {
    setColor(profile.window_color);
  }, [profile.window_color]);

  const persist = React.useCallback(
    async (hex: string) => {
      try {
        await invoke("update_profile_window_color", {
          profileId: profile.id,
          windowColor: hex,
        });
      } catch {
        setColor(profile.window_color);
      }
    },
    [profile.id, profile.window_color],
  );

  return (
    <Popover
      onOpenChange={(open) => {
        if (!open && color && color !== profile.window_color)
          void persist(color);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("profileInfo.fields.windowColor")}
          title={t("profileInfo.fields.windowColor")}
          className="size-9 shrink-0 cursor-pointer rounded-lg border shadow-sm ring-offset-background transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          style={{ backgroundColor: color ?? DEFAULT_SWATCH_COLOR }}
        />
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={6} className="w-[264px] p-3">
        <ColorPicker
          className="rounded-md border bg-background p-3 shadow-sm"
          value={color ?? DEFAULT_SWATCH_COLOR}
          onColorChange={([r, g, b]) => {
            setColor(Color({ r, g, b }).hex().toLowerCase());
          }}
        >
          <ColorPickerSelection className="h-32 rounded" />
          <div className="mt-3 flex items-center gap-3">
            <ColorPickerEyeDropper />
            <ColorPickerHue className="flex-1" />
          </div>
          <div className="mt-3 flex items-center gap-2">
            <ColorPickerOutput />
            <ColorPickerFormat />
          </div>
        </ColorPicker>
      </PopoverContent>
    </Popover>
  );
}

export function ProfileInfoDialog({
  isOpen,
  onClose,
  profile,
  storedProxies,
  vpnConfigs,
  onOpenTrafficDialog,
  onAssignProfilesToGroup,
  onCopyCookiesToProfile,
  onOpenCookieManagement,
  onAssignExtensionGroup,
  onOpenBypassRules,
  onOpenDnsBlocklist,
  onCloneProfile,
  onDeleteProfile,
  crossOsUnlocked = false,
  isRunning = false,
  isDisabled = false,
  isCrossOs = false,
}: ProfileInfoDialogProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const [groupName, setGroupName] = React.useState<string | null>(null);
  const [extensionGroupName, setExtensionGroupName] = React.useState<
    string | null
  >(null);

  React.useEffect(() => {
    if (!isOpen || !profile?.group_id) {
      setGroupName(null);
      return;
    }
    void (async () => {
      try {
        const groups = await invoke<ProfileGroup[]>("get_groups");
        const group = groups.find((g) => g.id === profile.group_id);
        setGroupName(group?.name ?? null);
      } catch {
        setGroupName(null);
      }
    })();
  }, [isOpen, profile?.group_id]);

  React.useEffect(() => {
    if (!isOpen || !profile?.extension_group_id) {
      setExtensionGroupName(null);
      return;
    }
    void (async () => {
      try {
        const group = await invoke<{ name: string } | null>(
          "get_extension_group_for_profile",
          { profileId: profile.id },
        );
        setExtensionGroupName(group?.name ?? null);
      } catch {
        setExtensionGroupName(null);
      }
    })();
  }, [isOpen, profile?.extension_group_id, profile?.id]);

  React.useEffect(() => {
    if (!isOpen) {
      setCopied(false);
    }
  }, [isOpen]);

  if (!profile) return null;

  const ProfileIcon = getProfileIcon(profile);
  const hasCookieBackend = supportsCookieBackend(profile.browser);
  const isDeleteDisabled = isRunning;

  const proxyName = profile.proxy_id
    ? storedProxies.find((p) => p.id === profile.proxy_id)?.name
    : null;
  const vpnName = profile.vpn_id
    ? vpnConfigs.find((v) => v.id === profile.vpn_id)?.name
    : null;
  const networkLabel = vpnName
    ? t("profileInfo.network.vpnLabel", { name: vpnName })
    : proxyName
      ? t("profileInfo.network.proxyLabel", { name: proxyName })
      : t("profileInfo.values.none");

  const handleCopyId = async () => {
    try {
      await navigator.clipboard.writeText(profile.id);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      // ignore
    }
  };

  const handleAction = (action: () => void) => {
    onClose();
    action();
  };

  const hasTags = profile.tags && profile.tags.length > 0;
  const hasNote = !!profile.note;

  // Items in the settings tab `actions` list MUST only open another dialog
  // (or trigger a navigation/action that closes this one). Do NOT put inline
  // settings UI — inputs, toggles, save buttons — directly in this dialog's
  // settings tab. Each setting belongs in its own focused dialog (see
  // `ProfileBypassRulesDialog`, `ProfileDnsBlocklistDialog` for the pattern).
  // The settings tab is purely a navigation hub.
  interface ActionItem {
    // Stable, language-independent key used to map sidebar sections to actions.
    // The sidebar must NOT match on `label` — labels are translated, so English
    // substring matching hides sections for every non-English user.
    id?: string;
    icon: React.ReactNode;
    label: string;
    onClick: () => void;
    disabled?: boolean;
    destructive?: boolean;
    proBadge?: boolean;
    runningBadge?: boolean;
    hidden?: boolean;
  }

  const actions: ActionItem[] = [
    {
      id: "network",
      icon: <LuGlobe className="size-4" />,
      label: t("profiles.actions.viewNetwork"),
      onClick: () => {
        handleAction(() => onOpenTrafficDialog?.(profile.id));
      },
      disabled: isCrossOs,
    },
    {
      icon: <LuGroup className="size-4" />,
      label: t("profiles.actions.assignToGroup"),
      onClick: () => {
        handleAction(() => onAssignProfilesToGroup?.([profile.id]));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
    },
    {
      id: "fingerprint",
      icon: <LuFingerprint className="size-4" />,
      label: t("profiles.actions.changeFingerprint"),
      onClick: () => {
        /* Selecting the sidebar section itself renders FingerprintSectionInline. */
      },
      // Viewing and editing fingerprints both require an active paid plan.
      disabled: isDisabled || !crossOsUnlocked,
      proBadge: !crossOsUnlocked,
      runningBadge: isRunning,
    },
    {
      id: "cookiesCopy",
      icon: <LuCopy className="size-4" />,
      label: t("profiles.actions.copyCookiesToProfile"),
      onClick: () => {
        handleAction(() => onCopyCookiesToProfile?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: !hasCookieBackend || !onCopyCookiesToProfile,
    },
    {
      id: "cookiesManage",
      icon: <LuCookie className="size-4" />,
      label: t("profileInfo.actions.manageCookies"),
      onClick: () => {
        handleAction(() => onOpenCookieManagement?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
      hidden: !hasCookieBackend || !onOpenCookieManagement,
    },
    {
      icon: <LuSettings className="size-4" />,
      label: t("profiles.actions.clone"),
      onClick: () => {
        handleAction(() => onCloneProfile?.(profile));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
    },
    {
      id: "extension",
      icon: <LuPuzzle className="size-4" />,
      label: t("profileInfo.actions.assignExtensionGroup"),
      onClick: () => {
        handleAction(() => onAssignExtensionGroup?.([profile.id]));
      },
      disabled: isDisabled,
      runningBadge: isRunning,
    },
    {
      icon: <LuShieldCheck className="size-4" />,
      label: t("profileInfo.network.bypassRulesTitle"),
      onClick: () => {
        handleAction(() => onOpenBypassRules?.(profile));
      },
    },
    {
      icon: <LuShield className="size-4" />,
      label: t("dnsBlocklist.title"),
      onClick: () => {
        handleAction(() => onOpenDnsBlocklist?.(profile));
      },
    },
    {
      id: "delete",
      icon: <LuTrash2 className="size-4" />,
      label: t("profiles.actions.delete"),
      onClick: () => {
        handleAction(() => onDeleteProfile?.(profile));
      },
      disabled: isDeleteDisabled,
      destructive: true,
    },
  ];

  const visibleActions = actions.filter((a) => !a.hidden);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        hideClose
        className="flex h-[min(clamp(30rem,80vh,48rem),calc(100vh-3rem))] max-w-[min(60rem,calc(100%-4rem))] flex-col gap-0 overflow-hidden p-0"
      >
        {/* The dialog renders its own custom header, so the accessible title is
            visually hidden but present for screen readers (Radix requires it). */}
        <DialogTitle className="sr-only">{t("profileInfo.title")}</DialogTitle>
        <ProfileInfoLayout
          profile={profile}
          ProfileIcon={ProfileIcon}
          isRunning={isRunning}
          isDisabled={isDisabled}
          networkLabel={networkLabel}
          groupName={groupName}
          extensionGroupName={extensionGroupName}
          storedProxies={storedProxies}
          vpnConfigs={vpnConfigs}
          hasTags={hasTags}
          hasNote={hasNote}
          copied={copied}
          handleCopyId={handleCopyId}
          onClose={onClose}
          onCloneProfile={onCloneProfile}
          onKillProfile={undefined}
          visibleActions={visibleActions}
          t={t}
        />
      </DialogContent>
    </Dialog>
  );
}

interface ProfileInfoLayoutProps {
  profile: BrowserProfile;
  ProfileIcon: React.ComponentType<{ className?: string }>;
  isRunning: boolean;
  isDisabled: boolean;
  networkLabel: string;
  groupName: string | null;
  extensionGroupName: string | null;
  hasTags: boolean | undefined;
  hasNote: boolean;
  copied: boolean;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  handleCopyId: () => Promise<void>;
  onClose: () => void;
  onCloneProfile?: (profile: BrowserProfile) => void;
  onKillProfile?: (profile: BrowserProfile) => void;
  visibleActions: {
    id?: string;
    icon: React.ReactNode;
    label: string;
    onClick: () => void;
    disabled?: boolean;
    destructive?: boolean;
    proBadge?: boolean;
    runningBadge?: boolean;
  }[];
  t: (key: string, options?: Record<string, unknown>) => string;
}

type ProfileSection =
  | "overview"
  | "fingerprint"
  | "network"
  | "cookies"
  | "extensions"
  | "delete";

function ProfileInfoLayout({
  profile,
  ProfileIcon,
  isRunning,
  isDisabled,
  networkLabel,
  groupName,
  extensionGroupName,
  storedProxies,
  vpnConfigs,
  hasTags,
  hasNote,
  copied,
  handleCopyId,
  onClose,
  onCloneProfile,
  visibleActions,
  t,
}: ProfileInfoLayoutProps) {
  const [section, setSection] = React.useState<ProfileSection>("overview");
  const [isExportDialogOpen, setIsExportDialogOpen] = React.useState(false);
  const [passwordDialogMode, setPasswordDialogMode] =
    React.useState<PasswordDialogMode>("set");
  const [isPasswordDialogOpen, setIsPasswordDialogOpen] = React.useState(false);

  // Map sidebar items to existing actions by their stable, language-independent
  // `id`, so clicking a section triggers the existing dialog handler. Matching
  // on `label` would break for every non-English locale (the labels are
  // translated) and hide whole sections.
  const findAction = React.useCallback(
    (id: string) => visibleActions.find((a) => a.id === id),
    [visibleActions],
  );

  const deleteAction = findAction("delete");
  const fingerprintAction = findAction("fingerprint");
  const cookiesManageAction = findAction("cookiesManage");
  const cookiesCopyAction = findAction("cookiesCopy");
  const cookiesAction = cookiesManageAction ?? cookiesCopyAction;
  const extensionAction = findAction("extension");
  const _networkAction = findAction("network");

  // Cookie count is fetched at the layout level so the sidebar badge can
  // surface it without waiting for the user to open the Cookies section.
  // The effect deps must be primitive — `cookiesAction` is a new object
  // every render and using it directly here caused an infinite re-render
  // loop that froze the entire app when the dialog opened.
  // Skipped while running: the cookie DB is held by the browser and we
  // don't want to compete for its lock from the badge fetch.
  const cookiesSupported = !!cookiesAction;
  const [cookieCount, setCookieCount] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (!cookiesSupported || isRunning) {
      setCookieCount(null);
      return;
    }
    let mounted = true;
    void (async () => {
      try {
        const data = await invoke<{ total_count: number }>(
          "get_profile_cookie_stats",
          { profileId: profile.id },
        );
        if (mounted) setCookieCount(data.total_count);
      } catch {
        if (mounted) setCookieCount(null);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [profile.id, cookiesSupported, isRunning]);

  const sidebarItems: {
    id: ProfileSection;
    icon: React.ReactNode;
    label: string;
    badge?: string;
    destructive?: boolean;
    hidden?: boolean;
  }[] = [
    {
      id: "overview",
      icon: <LuClipboard className="size-3.5" />,
      label: t("profileInfo.sections.overview"),
    },
    {
      id: "fingerprint",
      icon: <LuFingerprint className="size-3.5" />,
      label: t("profileInfo.sections.fingerprint"),
      hidden: !fingerprintAction && profile.browser !== "clearcote",
    },
    {
      id: "network",
      icon: <LuGlobe className="size-3.5" />,
      label: t("profileInfo.sections.network"),
      badge: profile.proxy_id || profile.vpn_id ? networkLabel : undefined,
    },
    {
      id: "cookies",
      icon: <LuCookie className="size-3.5" />,
      label: t("profileInfo.sections.cookies"),
      badge:
        cookieCount !== null && cookieCount > 0
          ? cookieCount.toLocaleString()
          : undefined,
      hidden: !cookiesAction,
    },
    {
      id: "extensions",
      icon: <LuPuzzle className="size-3.5" />,
      label: t("profileInfo.sections.extensions"),
      badge: extensionGroupName ?? undefined,
      hidden: !extensionAction,
    },
  ];

  return (
    <>
      {/* Top bar */}
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <LuUsers className="size-3.5 shrink-0 text-muted-foreground" />
        <div className="flex min-w-0 flex-1 items-center gap-1.5 text-xs">
          <span className="font-semibold">
            {t("profileInfo.breadcrumbRoot")}
          </span>
          <span className="text-muted-foreground">/</span>
          <span className="truncate text-muted-foreground">{profile.name}</span>
        </div>
        {onCloneProfile && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 px-2 text-xs"
            disabled={isDisabled}
            onClick={() => onCloneProfile(profile)}
          >
            <LuCopy className="size-3" />
            {t("profileInfo.duplicate")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 gap-1.5 px-2 text-xs"
          onClick={() => setIsExportDialogOpen(true)}
        >
          <LuDownload className="size-3" />
          {t("exportProfile.title")}
        </Button>
        <button
          type="button"
          aria-label={t("common.buttons.close")}
          onClick={onClose}
          className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors duration-100 hover:bg-accent hover:text-accent-foreground"
        >
          <LuX className="size-3.5" />
        </button>
      </div>

      {/* Body */}
      <div className="flex min-h-0 flex-1">
        {/* Sidebar */}
        <nav className="flex w-44 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-2">
          {sidebarItems
            .filter((it) => !it.hidden)
            .map((it) => {
              const active = section === it.id;
              return (
                <button
                  key={it.id}
                  type="button"
                  onClick={() => setSection(it.id)}
                  className={cn(
                    "flex h-7 items-center gap-2 rounded-md px-2 text-left text-xs transition-colors duration-100",
                    active
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                >
                  <span className="shrink-0">{it.icon}</span>
                  <span className="flex-1 truncate">{it.label}</span>
                  {it.badge && (
                    <span className="max-w-[60px] truncate text-[9px] tracking-wide text-muted-foreground uppercase">
                      {it.badge}
                    </span>
                  )}
                </button>
              );
            })}
          {deleteAction && (
            <>
              <div className="my-1 h-px bg-border" />
              <button
                type="button"
                onClick={deleteAction.onClick}
                disabled={deleteAction.disabled}
                className="flex h-7 items-center gap-2 rounded-md px-2 text-xs text-destructive-text transition-colors duration-100 hover:bg-destructive/10 disabled:pointer-events-none disabled:opacity-50"
              >
                <LuTrash2 className="size-3.5 shrink-0" />
                <span className="flex-1 text-left">
                  {t("profileInfo.sections.delete")}
                </span>
              </button>
            </>
          )}
        </nav>

        {/* Main */}
        <div className="scroll-fade min-w-0 flex-1 overflow-y-auto p-4">
          {section === "overview" && (
            <div className="flex flex-col gap-3">
              {/* Hero */}
              <div className="flex items-center gap-3">
                <div className="shrink-0 rounded-lg bg-muted p-2.5">
                  <ProfileIcon className="size-7 text-foreground" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <h3 className="truncate text-base font-semibold">
                      {profile.name}
                    </h3>
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px]">
                    <span className="font-mono text-muted-foreground">
                      {profile.version}
                    </span>
                  </div>
                </div>
                <WindowColorSwatch profile={profile} />
              </div>

              {/* ID */}
              <div className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
                <span className="shrink-0 text-[10px] tracking-wide text-muted-foreground uppercase">
                  ID
                </span>
                <span className="flex-1 truncate font-mono text-xs">
                  {profile.id}
                </span>
                <button
                  type="button"
                  onClick={() => void handleCopyId()}
                  className="shrink-0 text-muted-foreground transition-colors hover:text-foreground"
                  aria-label={t("common.buttons.copy")}
                >
                  {copied ? (
                    <LuClipboardCheck className="size-3.5" />
                  ) : (
                    <LuClipboard className="size-3.5" />
                  )}
                </button>
              </div>

              {/* 2x2 cards */}
              <div className="grid grid-cols-2 gap-2">
                <InfoCard
                  label={t("profileInfo.fields.group")}
                  value={groupName ?? t("profileInfo.values.none")}
                />
                <InfoCard
                  label={t("profileInfo.fields.proxyVpn")}
                  value={networkLabel}
                />
                <InfoCard
                  label={t("profileInfo.fields.tags")}
                  value={
                    hasTags
                      ? (profile.tags ?? []).join(", ")
                      : t("profileInfo.values.none")
                  }
                />
                <InfoCard
                  label={t("profileInfo.fields.note")}
                  value={
                    hasNote
                      ? (profile.note ?? "")
                      : t("profileInfo.values.none")
                  }
                />
              </div>

              <div className="flex items-center justify-between rounded-md border border-border bg-muted/40 px-3 py-2">
                <div>
                  <div className="text-xs font-medium">
                    {t("profileInfo.security.title")}
                  </div>
                  <div className="text-[11px] text-muted-foreground">
                    {profile.password_protected
                      ? t("profileInfo.security.protected")
                      : t("profileInfo.security.unprotected")}
                  </div>
                </div>
                <div className="flex gap-1.5">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={isDisabled || isRunning}
                    onClick={() => {
                      setPasswordDialogMode(
                        profile.password_protected ? "change" : "set",
                      );
                      setIsPasswordDialogOpen(true);
                    }}
                  >
                    {profile.password_protected
                      ? t("profileInfo.security.change")
                      : t("profileInfo.security.enable")}
                  </Button>
                  {profile.password_protected && (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={isDisabled || isRunning}
                      onClick={() => {
                        setPasswordDialogMode("remove");
                        setIsPasswordDialogOpen(true);
                      }}
                    >
                      {t("profileInfo.security.remove")}
                    </Button>
                  )}
                </div>
              </div>

              {/* Activity */}
              <div className="mt-1 flex flex-col gap-1.5">
                <span className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  {t("profileInfo.sections.activity")}
                </span>
                <div className="grid grid-cols-2 gap-2">
                  <InfoCard
                    label={t("profileInfo.fields.created")}
                    value={
                      profile.created_at
                        ? new Date(profile.created_at * 1000).toLocaleString(
                            undefined,
                            { dateStyle: "medium", timeStyle: "short" },
                          )
                        : t("profileInfo.values.unknown")
                    }
                  />
                  <InfoCard
                    label={t("profileInfo.fields.lastLaunched")}
                    value={
                      isRunning
                        ? t("profileInfo.values.activeNow")
                        : profile.last_launch
                          ? formatRelativeTime(profile.last_launch)
                          : t("profileInfo.values.never")
                    }
                  />
                  <LocalDataTransferCard profileId={profile.id} t={t} />
                </div>
              </div>
            </div>
          )}

          {section === "fingerprint" && (
            <FingerprintSectionInline
              profile={profile}
              isDisabled={isDisabled}
              crossOsUnlocked={Boolean(
                // Re-derive: parent passes crossOsUnlocked but the layout
                // doesn't get it; we get it implicitly via fingerprintAction's
                // proBadge state. Default to false if action missing.
                fingerprintAction && !fingerprintAction.proBadge,
              )}
              onSaved={onClose}
              t={t}
            />
          )}

          {section === "network" && (
            <NetworkSectionInline
              profile={profile}
              storedProxies={storedProxies}
              vpnConfigs={vpnConfigs}
              isDisabled={isDisabled}
              t={t}
            />
          )}

          {section === "cookies" && (
            <CookiesSectionInline
              profile={profile}
              isRunning={isRunning}
              isDisabled={isDisabled}
              onCopyCookies={cookiesCopyAction?.onClick}
              onImportCookies={cookiesManageAction?.onClick}
              t={t}
            />
          )}

          {section === "extensions" && (
            <ExtensionsSectionInline
              profile={profile}
              isDisabled={isDisabled}
              t={t}
            />
          )}
        </div>
      </div>
      <ExportProfileDialog
        isOpen={isExportDialogOpen}
        onClose={() => setIsExportDialogOpen(false)}
        profile={profile}
      />
      <ProfilePasswordDialog
        isOpen={isPasswordDialogOpen}
        onClose={() => setIsPasswordDialogOpen(false)}
        profile={profile}
        mode={passwordDialogMode}
      />
    </>
  );
}

function _SectionPlaceholder({
  icon,
  title,
  description,
  actionLabel,
  onAction,
  disabled,
  hint,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  actionLabel: string;
  onAction: () => void;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        {icon}
        {title}
      </div>
      <p className="text-xs text-muted-foreground">{description}</p>
      {hint && (
        <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
          {hint}
        </div>
      )}
      <Button
        size="sm"
        onClick={onAction}
        disabled={disabled}
        className="h-7 self-start text-xs"
      >
        {actionLabel}
      </Button>
    </div>
  );
}

function _SectionAction({
  icon,
  label,
  onClick,
  disabled,
  destructive,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  destructive?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex h-9 items-center gap-2 rounded-md px-3 text-left text-xs transition-colors",
        destructive
          ? "text-destructive-text hover:bg-destructive/10"
          : "hover:bg-accent hover:text-accent-foreground",
        "disabled:pointer-events-none disabled:opacity-50",
      )}
    >
      {icon}
      <span className="flex-1">{label}</span>
      <LuChevronRight className="size-3.5 text-muted-foreground" />
    </button>
  );
}

function NetworkSectionInline({
  profile,
  storedProxies,
  vpnConfigs,
  isDisabled,
  t,
}: {
  profile: BrowserProfile;
  storedProxies: StoredProxy[];
  vpnConfigs: VpnConfig[];
  isDisabled: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  // Track the effective selection so the dropdown reflects the last save
  // even before the parent `profile` prop refreshes from a backend event.
  const [proxyId, setProxyId] = React.useState<string | null>(
    profile.proxy_id ?? null,
  );
  const [vpnId, setVpnId] = React.useState<string | null>(
    profile.vpn_id ?? null,
  );

  React.useEffect(() => {
    setProxyId(profile.proxy_id ?? null);
    setVpnId(profile.vpn_id ?? null);
  }, [profile.proxy_id, profile.vpn_id]);

  const onProxyChange = async (value: string) => {
    const nextId = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_profile_proxy", {
        profileId: profile.id,
        proxyId: nextId,
      });
      // Clearing the proxy implicitly clears any VPN binding too on the
      // backend, but we mirror it locally for an immediate visual.
      setProxyId(nextId);
      if (nextId !== null) setVpnId(null);
    } catch (e) {
      setError(translateBackendError(t as never, e));
    } finally {
      setIsSaving(false);
    }
  };

  const onVpnChange = async (value: string) => {
    const nextId = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_profile_vpn", {
        profileId: profile.id,
        vpnId: nextId,
      });
      setVpnId(nextId);
      if (nextId !== null) setProxyId(null);
    } catch (e) {
      setError(translateBackendError(t as never, e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuGlobe className="size-4" />
        {t("profileInfo.sections.network")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.network")}
      </p>

      <div className="flex items-center gap-2">
        <span className="w-12 shrink-0 text-[10px] tracking-wide text-muted-foreground uppercase">
          {t("profileInfo.fields.proxy")}
        </span>
        <Select
          value={proxyId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onProxyChange(v);
          }}
        >
          <SelectTrigger className="h-7 flex-1 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {storedProxies.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex items-center gap-2">
        <span className="w-12 shrink-0 text-[10px] tracking-wide text-muted-foreground uppercase">
          {t("profileInfo.fields.vpn")}
        </span>
        <Select
          value={vpnId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onVpnChange(v);
          }}
        >
          <SelectTrigger className="h-7 flex-1 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {vpnConfigs.map((v) => (
              <SelectItem key={v.id} value={v.id}>
                {v.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {error && <p className="text-xs text-destructive-text">{error}</p>}
    </div>
  );
}

function ExtensionsSectionInline({
  profile,
  isDisabled,
  t,
}: {
  profile: BrowserProfile;
  isDisabled: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type ExtensionGroupOption = { id: string; name: string };
  const [groups, setGroups] = React.useState<ExtensionGroupOption[]>([]);
  const [groupId, setGroupId] = React.useState<string | null>(
    profile.extension_group_id ?? null,
  );
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    setGroupId(profile.extension_group_id ?? null);
  }, [profile.extension_group_id]);

  React.useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    const load = async () => {
      try {
        const data = await invoke<ExtensionGroupOption[]>(
          "list_extension_groups",
        );
        if (mounted) setGroups(data);
      } catch (e) {
        if (mounted) setError(translateBackendError(t as never, e));
      }
    };
    void load();
    void listen("extensions-changed", () => {
      void load();
    }).then((u) => {
      if (mounted) unlisten = u;
      else u();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, [t]);

  const onChange = async (value: string) => {
    const next = value === "__none__" ? null : value;
    setIsSaving(true);
    setError(null);
    try {
      await invoke("assign_extension_group_to_profile", {
        profileId: profile.id,
        extensionGroupId: next,
      });
      setGroupId(next);
    } catch (e) {
      setError(translateBackendError(t as never, e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <LuPuzzle className="size-4" />
        {t("profileInfo.sections.extensions")}
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.extensions")}
      </p>
      <div className="flex items-center gap-2">
        <span className="w-16 shrink-0 text-[10px] tracking-wide text-muted-foreground uppercase">
          {t("profileInfo.fields.extensionGroup")}
        </span>
        <Select
          value={groupId ?? "__none__"}
          disabled={isDisabled || isSaving}
          onValueChange={(v) => {
            void onChange(v);
          }}
        >
          <SelectTrigger className="h-7 flex-1 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__none__">
              {t("profileInfo.values.none")}
            </SelectItem>
            {groups.map((g) => (
              <SelectItem key={g.id} value={g.id}>
                {g.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {error && <p className="text-xs text-destructive-text">{error}</p>}
    </div>
  );
}

function CookiesSectionInline({
  profile,
  isRunning,
  isDisabled,
  onCopyCookies,
  onImportCookies,
  t,
}: {
  profile: BrowserProfile;
  isRunning: boolean;
  isDisabled: boolean;
  onCopyCookies?: () => void;
  onImportCookies?: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  type CookieStats = {
    profile_id: string;
    browser_type: string;
    total_count: number;
    domains: { domain: string; count: number }[];
  };
  const [stats, setStats] = React.useState<CookieStats | null>(null);
  const [isLoading, setIsLoading] = React.useState(!isRunning);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (isRunning) {
      setStats(null);
      setIsLoading(false);
      setError(null);
      return;
    }
    let mounted = true;
    setIsLoading(true);
    setError(null);
    void (async () => {
      try {
        const data = await invoke<CookieStats>("get_profile_cookie_stats", {
          profileId: profile.id,
        });
        if (mounted) setStats(data);
      } catch (e) {
        if (mounted) setError(translateBackendError(t as never, e));
      } finally {
        if (mounted) setIsLoading(false);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [profile.id, isRunning, t]);

  const [isExporting, setIsExporting] = React.useState(false);

  // Export all of this profile's cookies in one of the same formats import
  // accepts (JSON or Netscape). The backend formats every cookie; we just pick
  // a destination file.
  const handleExport = React.useCallback(
    async (format: "json" | "netscape") => {
      setIsExporting(true);
      try {
        const content = await invoke<string>("export_profile_cookies", {
          profileId: profile.id,
          format,
        });
        const ext = format === "json" ? "json" : "txt";
        const filePath = await save({
          defaultPath: `${profile.name}_cookies.${ext}`,
          filters: [
            {
              name: format === "json" ? "JSON" : "Text",
              extensions: [ext],
            },
          ],
        });
        if (!filePath) return;
        await writeTextFile(filePath, content);
        showSuccessToast(t("cookies.export.success"));
      } catch (e) {
        showErrorToast(translateBackendError(t as never, e));
      } finally {
        setIsExporting(false);
      }
    },
    [profile.id, profile.name, t],
  );

  const domains = stats?.domains ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuCookie className="size-4" />
          {t("profileInfo.sections.cookies")}
        </div>
        <div className="flex items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className="h-7 gap-1.5"
                disabled={
                  isDisabled ||
                  isRunning ||
                  isExporting ||
                  !stats ||
                  stats.total_count === 0
                }
              >
                <LuDownload className="size-3.5" />
                {t("common.buttons.export")}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                onClick={() => {
                  void handleExport("json");
                }}
              >
                {t("cookies.export.json")}
              </DropdownMenuItem>
              <DropdownMenuItem
                onClick={() => {
                  void handleExport("netscape");
                }}
              >
                {t("cookies.export.netscape")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {onImportCookies && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5"
              disabled={isDisabled || isRunning}
              onClick={onImportCookies}
            >
              <LuUpload className="size-3.5" />
              {t("common.buttons.import")}
            </Button>
          )}
          {onCopyCookies && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5"
              disabled={isDisabled}
              onClick={onCopyCookies}
            >
              <LuCopy className="size-3.5" />
              {t("common.buttons.copy")}
            </Button>
          )}
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("profileInfo.sectionDesc.cookies")}
      </p>
      {isRunning ? (
        <div className="rounded-md border border-border bg-muted/40 px-3 py-2">
          <p className="text-xs text-muted-foreground">
            {t("profileInfo.cookies.runningNotice")}
          </p>
        </div>
      ) : (
        <>
          <div className="rounded-md border border-border bg-muted/40 px-3 py-2">
            <p className="text-[10px] tracking-wide text-muted-foreground uppercase">
              {t("profileInfo.fields.cookieCount")}
            </p>
            <p className="mt-0.5 text-sm">
              {isLoading
                ? t("profileInfo.values.loading")
                : stats
                  ? stats.total_count.toLocaleString()
                  : "—"}
            </p>
          </div>
          {domains.length > 0 && (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-md border border-border bg-muted/40">
              <p className="shrink-0 border-b border-border px-3 py-2 text-[10px] tracking-wide text-muted-foreground uppercase">
                {t("profileInfo.cookies.domainsHeader", {
                  count: domains.length,
                })}
              </p>
              <ul className="flex-1 space-y-1 overflow-y-auto px-3 py-2 text-xs">
                {domains.map((d) => (
                  <li
                    key={d.domain}
                    className="flex items-center justify-between gap-2"
                  >
                    <span className="truncate font-mono">{d.domain}</span>
                    <span className="text-muted-foreground tabular-nums">
                      {d.count}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {error && <p className="text-xs text-destructive-text">{error}</p>}
        </>
      )}
    </div>
  );
}

/**
 * Camoufox generates fingerprints locally via a Bayesian-network model (no
 * headless-launch device synthesis, no cloud identity concept), so
 * there is nothing to edit field-by-field here yet — only whether a stored
 * fingerprint is kept or cleared so the next launch generates a fresh one.
 */
const SCREEN_SIZE_FIELDS = [
  ["screen_min_width", "createProfile.screen.minWidth"],
  ["screen_max_width", "createProfile.screen.maxWidth"],
  ["screen_min_height", "createProfile.screen.minHeight"],
  ["screen_max_height", "createProfile.screen.maxHeight"],
] as const;

function CamoufoxFingerprintSection({
  profile,
  onSaved,
  t,
}: {
  profile: BrowserProfile;
  onSaved: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const config = profile.camoufox_config ?? {};
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [success, setSuccess] = React.useState<string | null>(null);

  const hasFingerprint = Boolean(config.fingerprint);
  const [presetTabs, setPresetTabs] = React.useState<string[]>(
    config.preset_tabs ?? [],
  );
  const [customPresetSites, setCustomPresetSites] = React.useState<PrelaunchSite[]>([]);
  const [os, setOs] = React.useState<CamoufoxOS>(config.os ?? "windows");
  const [screenSizes, setScreenSizes] = React.useState<
    Record<(typeof SCREEN_SIZE_FIELDS)[number][0], number | undefined>
  >({
    screen_min_width: config.screen_min_width,
    screen_max_width: config.screen_max_width,
    screen_min_height: config.screen_min_height,
    screen_max_height: config.screen_max_height,
  });

  React.useEffect(() => {
    void invoke<{
      global_preset_tabs?: string[];
      global_prelaunch_sites?: PrelaunchSite[];
    }>("get_app_settings")
      .then((settings) => {
        const named = (settings.global_prelaunch_sites ?? []).filter(
          (site) => site.name.trim() && /^https?:\/\/[^\s]+$/i.test(site.url),
        );
        const legacy = (settings.global_preset_tabs ?? [])
          .filter((url) => /^https?:\/\/[^\s]+$/i.test(url))
          .map((url) => ({ name: new URL(url).hostname, url }));
        setCustomPresetSites([
          ...named,
          ...legacy.filter((site) => !named.some((item) => item.url === site.url)),
        ]);
      })
      .catch(() => setCustomPresetSites([]));
  }, []);

  const saveConfig = async (
    patch: Partial<CamoufoxConfig>,
    rollback: () => void,
  ): Promise<boolean> => {
    setIsSaving(true);
    setError(null);
    setSuccess(null);
    try {
      const nextConfig: CamoufoxConfig = { ...config, ...patch };
      await invoke("update_camoufox_config", {
        profileId: profile.id,
        config: nextConfig,
      });
      onSaved();
      return true;
    } catch (e) {
      rollback();
      setError(translateBackendError(t as never, e));
      return false;
    } finally {
      setIsSaving(false);
    }
  };

  const changeOs = (value: string) => {
    const previous = os;
    const next = value as CamoufoxOS;
    setOs(next);
    void saveConfig({ os: next }, () => setOs(previous));
  };

  const commitScreenSize = (key: (typeof SCREEN_SIZE_FIELDS)[number][0]) => {
    const previous = config[key];
    if (screenSizes[key] === previous) return;
    void saveConfig({ [key]: screenSizes[key] }, () =>
      setScreenSizes((prev) => ({ ...prev, [key]: previous })),
    );
  };

  const toggleFlag = (
    key: "randomize_fingerprint_on_launch" | "block_images" | "block_webrtc" | "block_webgl" | "geoip",
    checked: boolean,
  ) => {
    void saveConfig({ [key]: checked }, () => {
      /* On failure the component re-reads from `profile` on the next render
         via its parent's refresh, so no local toggle state to roll back. */
    });
  };

  const onRegenerate = async () => {
    const ok = await saveConfig({ fingerprint: undefined }, () => {});
    if (ok) setSuccess(t("profileInfo.fingerprint.camoufoxRegenerateQueued"));
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuFingerprint className="size-4" />
          {t("profileInfo.sections.fingerprint")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("profileInfo.fingerprint.camoufoxDescription")}
        </p>
        <div>
          <Button
            variant="outline"
            size="sm"
            disabled={isSaving || !hasFingerprint}
            onClick={() => void onRegenerate()}
          >
            {t("profileInfo.fingerprint.camoufoxRegenerateButton")}
          </Button>
        </div>
      </div>

      <div className="space-y-2">
        <Label htmlFor="camoufox-profile-os" className="text-xs">
          {t("createProfile.os.label")}
        </Label>
        <Select value={os} onValueChange={changeOs} disabled={isSaving}>
          <SelectTrigger id="camoufox-profile-os">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="windows">{t("createProfile.os.windows")}</SelectItem>
            <SelectItem value="macos">{t("createProfile.os.macos")}</SelectItem>
            <SelectItem value="linux">{t("createProfile.os.linux")}</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="grid grid-cols-2 gap-3">
        {SCREEN_SIZE_FIELDS.map(([key, label]) => (
          <div className="space-y-2" key={key}>
            <Label htmlFor={`camoufox-profile-${key}`} className="text-xs">
              {t(label)}
            </Label>
            <Input
              id={`camoufox-profile-${key}`}
              type="number"
              min={1}
              disabled={isSaving}
              value={screenSizes[key] ?? ""}
              onChange={(event) =>
                setScreenSizes((prev) => ({
                  ...prev,
                  [key]: event.target.value
                    ? Number(event.target.value)
                    : undefined,
                }))
              }
              onBlur={() => commitScreenSize(key)}
              placeholder={t("createProfile.screen.autoPlaceholder")}
            />
          </div>
        ))}
      </div>

      <div className="flex items-center justify-between gap-4">
        <Label htmlFor="camoufox-profile-randomize" className="text-xs">
          {t("createProfile.randomizeFingerprint.label")}
        </Label>
        <Checkbox
          id="camoufox-profile-randomize"
          disabled={isSaving}
          checked={config.randomize_fingerprint_on_launch ?? false}
          onCheckedChange={(checked) =>
            toggleFlag("randomize_fingerprint_on_launch", checked === true)
          }
        />
      </div>

      <div className="flex items-center justify-between">
        <Label htmlFor="camoufox-profile-block-images" className="text-xs">
          {t("createProfile.blockImages")}
        </Label>
        <Checkbox
          id="camoufox-profile-block-images"
          disabled={isSaving}
          checked={config.block_images ?? false}
          onCheckedChange={(checked) => toggleFlag("block_images", checked === true)}
        />
      </div>

      <div className="flex items-center justify-between">
        <Label htmlFor="camoufox-profile-block-webrtc" className="text-xs">
          {t("createProfile.blockWebrtc")}
        </Label>
        <Checkbox
          id="camoufox-profile-block-webrtc"
          disabled={isSaving}
          checked={config.block_webrtc ?? false}
          onCheckedChange={(checked) => toggleFlag("block_webrtc", checked === true)}
        />
      </div>

      <div className="flex items-center justify-between">
        <Label htmlFor="camoufox-profile-block-webgl" className="text-xs">
          {t("createProfile.blockWebgl")}
        </Label>
        <Checkbox
          id="camoufox-profile-block-webgl"
          disabled={isSaving}
          checked={config.block_webgl ?? false}
          onCheckedChange={(checked) => toggleFlag("block_webgl", checked === true)}
        />
      </div>

      <div className="flex items-center justify-between">
        <Label htmlFor="camoufox-profile-geoip" className="text-xs">
          {t("createProfile.geoip")}
        </Label>
        <Checkbox
          id="camoufox-profile-geoip"
          disabled={isSaving}
          checked={config.geoip !== false}
          onCheckedChange={(checked) => toggleFlag("geoip", checked === true)}
        />
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}
      {success && <p className="text-xs text-muted-foreground">{success}</p>}

      <div className="flex flex-col gap-2 border-t pt-3">
        <Label className="text-xs font-semibold">
          {t("createProfile.presetTabs.label")}
        </Label>
        <PrelaunchSitesSelector
          idPrefix="profile-preset"
          t={t}
          customSites={customPresetSites}
          selectedUrls={presetTabs}
          disabled={isSaving}
          onChange={(urls) => {
            const previous = presetTabs;
            setPresetTabs(urls);
            void saveConfig({ preset_tabs: urls }, () => setPresetTabs(previous));
          }}
        />
      </div>
    </div>
  );
}

function FingerprintSectionInline({
  profile,
  onSaved,
  t,
}: {
  profile: BrowserProfile;
  isDisabled: boolean;
  crossOsUnlocked: boolean;
  onSaved: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  if (profile.browser === "clearcote") {
    return <ClearcoteSitesSection profile={profile} onSaved={onSaved} t={t} />;
  }
  return (
    <CamoufoxFingerprintSection profile={profile} onSaved={onSaved} t={t} />
  );
}

function ClearcoteSitesSection({
  profile,
  onSaved,
  t,
}: {
  profile: BrowserProfile;
  onSaved: () => void;
  t: (key: string, options?: Record<string, unknown>) => string;
}) {
  const browserConfig = profile.browser_config ?? {};
  const storedTabs = browserConfig.preset_tabs;
  const [presetTabs, setPresetTabs] = React.useState<string[]>(
    Array.isArray(storedTabs)
      ? storedTabs.filter((url): url is string => typeof url === "string")
      : [],
  );
  const [customPresetSites, setCustomPresetSites] = React.useState<PrelaunchSite[]>([]);
  const storedPlatform = browserConfig.fingerprint_platform;
  const [platform, setPlatform] = React.useState<CamoufoxOS | "auto">(
    storedPlatform === "windows" ||
      storedPlatform === "macos" ||
      storedPlatform === "linux"
      ? storedPlatform
      : "auto",
  );
  const storedSeed = browserConfig.fingerprint_seed;
  const [seed, setSeed] = React.useState<string>(
    typeof storedSeed === "string" ? storedSeed : "",
  );
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    void invoke<{
      global_preset_tabs?: string[];
      global_prelaunch_sites?: PrelaunchSite[];
    }>("get_app_settings")
      .then((settings) => {
        const named = (settings.global_prelaunch_sites ?? []).filter(
          (site) => site.name.trim() && /^https?:\/\/[^\s]+$/i.test(site.url),
        );
        const legacy = (settings.global_preset_tabs ?? [])
          .filter((url) => /^https?:\/\/[^\s]+$/i.test(url))
          .map((url) => ({ name: new URL(url).hostname, url }));
        setCustomPresetSites([
          ...named,
          ...legacy.filter((site) => !named.some((item) => item.url === site.url)),
        ]);
      })
      .catch(() => setCustomPresetSites([]));
  }, []);

  const saveConfig = async (
    patch: Record<string, unknown>,
    rollback: () => void,
  ) => {
    setIsSaving(true);
    setError(null);
    try {
      await invoke("update_browser_config", {
        profileId: profile.id,
        config: { ...browserConfig, ...patch },
      });
      onSaved();
    } catch (err) {
      rollback();
      setError(translateBackendError(t as never, err));
    } finally {
      setIsSaving(false);
    }
  };

  const changePlatform = (value: string) => {
    const previous = platform;
    const next = value === "auto" ? undefined : (value as CamoufoxOS);
    setPlatform(next ?? "auto");
    void saveConfig({ fingerprint_platform: next }, () => setPlatform(previous));
  };

  const commitSeed = () => {
    const trimmed = seed.trim();
    if (trimmed === (typeof storedSeed === "string" ? storedSeed : "")) return;
    const previous = typeof storedSeed === "string" ? storedSeed : "";
    void saveConfig({ fingerprint_seed: trimmed || undefined }, () =>
      setSeed(previous),
    );
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuFingerprint className="size-4" />
          {t("createProfile.engineSettings.title")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("createProfile.engineSettings.description")}
        </p>
        <div className="space-y-2">
          <Label htmlFor="clearcote-profile-platform" className="text-xs">
            {t("createProfile.engineSettings.platform")}
          </Label>
          <Select
            value={platform}
            onValueChange={changePlatform}
            disabled={isSaving}
          >
            <SelectTrigger id="clearcote-profile-platform">
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
          <Label htmlFor="clearcote-profile-seed" className="text-xs">
            {t("createProfile.engineSettings.seed")}
          </Label>
          <Input
            id="clearcote-profile-seed"
            value={seed}
            disabled={isSaving}
            onChange={(event) => setSeed(event.target.value)}
            onBlur={commitSeed}
            placeholder={t("createProfile.engineSettings.seedPlaceholder")}
          />
          <p className="text-xs text-muted-foreground">
            {t("createProfile.engineSettings.seedHint")}
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-2 border-t pt-3">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <LuGlobe className="size-4" />
          {t("createProfile.presetTabs.label")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("createProfile.presetTabs.description")}
        </p>
        <PrelaunchSitesSelector
          idPrefix="clearcote-profile-preset"
          t={t}
          customSites={customPresetSites}
          selectedUrls={presetTabs}
          disabled={isSaving}
          onChange={(urls) => {
            const previous = presetTabs;
            setPresetTabs(urls);
            void saveConfig({ preset_tabs: urls }, () => setPresetTabs(previous));
          }}
        />
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

interface ProfileDnsBlocklistDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  currentLevel: string | null;
}

export function ProfileDnsBlocklistDialog({
  isOpen,
  onClose,
  profileId,
  currentLevel,
}: ProfileDnsBlocklistDialogProps) {
  const { t } = useTranslation();
  const [level, setLevel] = React.useState(currentLevel ?? "");
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    if (isOpen) {
      setLevel(currentLevel ?? "");
    }
  }, [isOpen, currentLevel]);

  const handleSave = async () => {
    if (!profileId) return;
    setIsSaving(true);
    try {
      await invoke("update_profile_dns_blocklist", {
        profileId,
        dnsBlocklist: level || null,
      });
      onClose();
    } catch (err) {
      console.error("Failed to update DNS blocklist:", err);
    } finally {
      setIsSaving(false);
    }
  };

  const options = [
    { value: "", label: t("dnsBlocklist.none") },
    ...DNS_BLOCKLIST_LEVELS.map((l) => ({
      value: l.value as string,
      label: t(l.labelKey),
    })),
  ];

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-xs">
        <DialogHeader>
          <DialogTitle>{t("dnsBlocklist.title")}</DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {t("dnsBlocklist.settingsDescription")}{" "}
          <a
            href="https://github.com/hagezi/dns-blocklists"
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary-text hover:underline"
          >
            {t("common.buttons.moreInfo")}
          </a>
        </p>
        <div className="space-y-1">
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setLevel(option.value)}
              className={`w-full text-left px-3 py-2 rounded-md text-sm transition-colors ${
                level === option.value
                  ? "border border-primary/30 bg-primary/10 text-primary-text"
                  : "border border-transparent hover:bg-accent hover:text-accent-foreground"
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <Button
          onClick={() => void handleSave()}
          disabled={isSaving || level === (currentLevel ?? "")}
          className="w-full"
        >
          {t("common.buttons.save")}
        </Button>
      </DialogContent>
    </Dialog>
  );
}

interface ProfileBypassRulesDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  initialRules?: string[];
}

export function ProfileBypassRulesDialog({
  isOpen,
  onClose,
  profileId,
  initialRules,
}: ProfileBypassRulesDialogProps) {
  const { t } = useTranslation();
  const [bypassRules, setBypassRules] = React.useState<string[]>([]);
  const [newRule, setNewRule] = React.useState("");

  React.useEffect(() => {
    if (isOpen) {
      setBypassRules(initialRules ?? []);
      setNewRule("");
    }
  }, [isOpen, initialRules]);

  const updateBypassRules = async (rules: string[]) => {
    if (!profileId) return;
    try {
      await invoke("update_profile_proxy_bypass_rules", {
        profileId,
        rules,
      });
      setBypassRules(rules);
    } catch {
      // ignore
    }
  };

  const handleAddRule = () => {
    const trimmed = newRule.trim();
    if (!trimmed || bypassRules.includes(trimmed)) return;
    const updated = [...bypassRules, trimmed];
    setNewRule("");
    void updateBypassRules(updated);
  };

  const handleRemoveRule = (rule: string) => {
    void updateBypassRules(bypassRules.filter((r) => r !== rule));
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex max-h-[80vh] flex-col sm:max-w-lg">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("profileInfo.network.bypassRulesTitle")}</DialogTitle>
        </DialogHeader>
        <ScrollArea className="min-h-0 flex-1">
          <div className="flex flex-col gap-3 py-2">
            <p className="text-sm text-muted-foreground">
              {t("profileInfo.network.bypassRulesDescription")}
            </p>
            <div className="flex gap-2">
              <Input
                value={newRule}
                onChange={(e) => {
                  setNewRule(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleAddRule();
                }}
                placeholder={t("profileInfo.network.rulePlaceholder")}
                className="flex-1 text-sm"
              />
              <Button
                size="sm"
                onClick={handleAddRule}
                disabled={!newRule.trim()}
              >
                <LuPlus className="mr-1 size-4" />
                {t("profileInfo.network.addRule")}
              </Button>
            </div>
            {bypassRules.length === 0 ? (
              <p className="py-2 text-sm text-muted-foreground">
                {t("profileInfo.network.noRules")}
              </p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {bypassRules.map((rule) => (
                  <div
                    key={rule}
                    className="flex items-center justify-between gap-2 rounded-md bg-muted px-3 py-1.5 text-sm"
                  >
                    <span className="truncate font-mono text-xs">{rule}</span>
                    <button
                      type="button"
                      onClick={() => {
                        handleRemoveRule(rule);
                      }}
                      className="shrink-0 text-muted-foreground transition-colors hover:text-destructive-text"
                    >
                      <LuX className="size-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
            <p className="text-xs text-muted-foreground">
              {t("profileInfo.network.ruleTypes")}
            </p>
          </div>
        </ScrollArea>
        <DialogFooter className="shrink-0">
          <Button variant="outline" onClick={onClose}>
            {t("common.buttons.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
