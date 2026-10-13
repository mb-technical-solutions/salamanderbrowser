"use client";

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import { translateBackendError } from "@/lib/backend-errors";
import { fireSprinkleConfetti } from "@/lib/confetti";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

interface ProfileExportWarning {
  profile_name: string;
  skipped_linked_extensions: string[];
}

interface ExportProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** Single-profile export. */
  profile?: BrowserProfile | null;
  /** Bulk export — still one combined archive, one password, one file. */
  profiles?: BrowserProfile[];
}

/** Sanitize a profile name into a safe default filename. */
function fileNameFor(profile: BrowserProfile): string {
  const safe = profile.name.replace(/[\\/:*?"<>|]/g, "_").trim() || "profile";
  return `${safe}.salamander-profile`;
}

// Mirrors `profile::password::MIN_PASSWORD_LEN` — the backend is the
// authority (PASSWORD_TOO_SHORT surfaces if this ever drifts), this is just
// for disabling the button before a doomed request round-trips.
const MIN_PASSWORD_LEN = 8;

export function ExportProfileDialog({
  isOpen,
  onClose,
  profile,
  profiles,
}: ExportProfileDialogProps) {
  const { t } = useTranslation();
  const [password, setPassword] = React.useState("");
  const [includeProxy, setIncludeProxy] = React.useState(false);
  const [includeVpn, setIncludeVpn] = React.useState(false);
  const [isExporting, setIsExporting] = React.useState(false);

  const targets = profile ? [profile] : (profiles ?? []);
  const isBulk = !profile && targets.length > 1;
  const anyHasProxy = targets.some((p) => Boolean(p.proxy_id));
  const anyHasVpn = targets.some((p) => Boolean(p.vpn_id));

  React.useEffect(() => {
    if (!isOpen) return;
    setPassword("");
    setIncludeProxy(false);
    setIncludeVpn(false);
  }, [isOpen]);

  if (targets.length === 0) return null;

  const handleExport = async () => {
    setIsExporting(true);
    try {
      const outputPath = await save({
        title: t("exportProfile.saveDialogTitle"),
        defaultPath: isBulk
          ? t("exportProfile.bulkFileName", { count: targets.length })
          : fileNameFor(targets[0]),
        filters: [
          { name: "Salamander Profile", extensions: ["salamander-profile"] },
        ],
      });
      if (!outputPath) {
        setIsExporting(false);
        return;
      }

      const warnings = await invoke<ProfileExportWarning[]>(
        "export_profiles_archive",
        {
          profileIds: targets.map((p) => p.id),
          archivePassword: password,
          includeProxyCredentials: includeProxy,
          includeVpnCredentials: includeVpn,
          outputPath,
        },
      );

      for (const warning of warnings) {
        if (warning.skipped_linked_extensions.length > 0) {
          showErrorToast(
            t("exportProfile.exportSkippedExtensions", {
              profile: warning.profile_name,
              names: warning.skipped_linked_extensions.join(", "),
            }),
          );
        }
      }

      showSuccessToast(
        isBulk
          ? t("exportProfile.bulkExportSuccess", { count: targets.length })
          : t("exportProfile.exportSuccess", { name: targets[0].name }),
      );
      fireSprinkleConfetti();
      onClose();
    } catch (error) {
      console.error("Failed to export profile(s):", error);
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsExporting(false);
    }
  };

  const canExport = password.trim().length >= MIN_PASSWORD_LEN;

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {isBulk
              ? t("exportProfile.bulkTitle", { count: targets.length })
              : t("exportProfile.title")}
          </DialogTitle>
          <DialogDescription>
            {isBulk ? t("exportProfile.bulkHint") : t("exportProfile.hint")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <Label htmlFor="export-archive-password" className="mb-2">
              {t("exportProfile.passwordLabel")}
            </Label>
            <Input
              id="export-archive-password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("exportProfile.passwordPlaceholder")}
              disabled={isExporting}
              autoComplete="new-password"
            />
            <p className="mt-1.5 text-xs text-muted-foreground">
              {t("exportProfile.passwordHint")}
            </p>
          </div>

          {anyHasProxy && (
            <div className="flex items-start gap-2">
              <Checkbox
                id="export-include-proxy"
                checked={includeProxy}
                onCheckedChange={(checked) => setIncludeProxy(checked === true)}
                disabled={isExporting}
              />
              <div>
                <Label htmlFor="export-include-proxy" className="font-normal">
                  {t("exportProfile.includeProxyLabel")}
                </Label>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("exportProfile.includeProxyHint")}
                </p>
              </div>
            </div>
          )}

          {anyHasVpn && (
            <div className="flex items-start gap-2">
              <Checkbox
                id="export-include-vpn"
                checked={includeVpn}
                onCheckedChange={(checked) => setIncludeVpn(checked === true)}
                disabled={isExporting}
              />
              <div>
                <Label htmlFor="export-include-vpn" className="font-normal">
                  {t("exportProfile.includeVpnLabel")}
                </Label>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("exportProfile.includeVpnHint")}
                </p>
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isExporting}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            onClick={() => void handleExport()}
            isLoading={isExporting}
            disabled={!canExport}
          >
            {isBulk
              ? t("exportProfile.bulkExportButton", { count: targets.length })
              : t("exportProfile.exportButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
