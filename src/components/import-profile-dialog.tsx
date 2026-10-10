"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FaFolder } from "react-icons/fa";
import { LuChevronRight } from "react-icons/lu";
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
import { cn } from "@/lib/utils";

interface ImportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  crossOsUnlocked?: boolean;
  subPage?: boolean;
}

/**
 * Import an existing Camoufox (Firefox-family) profile folder — e.g. one
 * saved via the upstream Python `camoufox` library — as a new Salamander
 * profile. This is the only import path left after Wayfern's removal: the
 * old multi-browser (Chrome/Brave/Edge/Vivaldi) importer relied on
 * OS-keychain secret re-sealing that only made sense for Chromium's
 * `os_crypt`, and was deleted along with Wayfern rather than ported.
 */
export function ImportProfileDialog({
  isOpen,
  onClose,
  subPage,
}: ImportProfileDialogProps) {
  const { t } = useTranslation();

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

  const handleBrowseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("importProfile.selectFolderTitle"),
      });
      if (selected && typeof selected === "string") {
        setProfilePath(selected);
        if (!profileName.trim()) {
          const parts = selected.split(/[/\\]/).filter(Boolean);
          setProfileName(parts[parts.length - 1] ?? "");
        }
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
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

  const handleClose = () => {
    setProfilePath("");
    setProfileName("");
    setProxyAssignment("none");
    setVpnAssignment("none");
    setSelectedGroupId("none");
    setIsCreatingGroup(false);
    setNewGroupName("");
    setShowFingerprint(false);
    setFingerprintJson("");
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
                  onClick={() => void handleBrowseFolder()}
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
              <Label className="mb-2">{t("importProfile.proxyOptional")}</Label>
              <Select value={proxyAssignment} onValueChange={setProxyAssignment}>
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
                <Label className="mb-2">{t("importProfile.vpnOptional")}</Label>
                <Select value={vpnAssignment} onValueChange={setVpnAssignment}>
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
              <Label className="mb-2">{t("importProfile.groupOptional")}</Label>
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
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
