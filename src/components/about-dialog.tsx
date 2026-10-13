"use client";

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuArrowLeft, LuExternalLink, LuSearch } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import { StepTransition } from "@/components/ui/step-transition";
import licenses from "@/generated/licenses.json";
import xraySource from "@/generated/xray-source.json";
import { RippleButton } from "./ui/ripple";

interface AboutDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

interface SystemInfo {
  app_version: string;
  os: string;
  arch: string;
  portable: boolean;
}

type AboutView = "about" | "licenses";

export function AboutDialog({ isOpen, onClose }: AboutDialogProps) {
  const { t } = useTranslation();
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [view, setView] = useState<AboutView>("about");
  const [licenseQuery, setLicenseQuery] = useState("");

  const restoreLicensesButtonFocusRef = useRef(false);

  useEffect(() => {
    if (!isOpen) return;
    invoke<SystemInfo>("get_system_info")
      .then(setSystemInfo)
      .catch(() => {
        setSystemInfo(null);
      });
  }, [isOpen]);

  const handleClose = useCallback(() => {
    setView("about");
    setLicenseQuery("");
    restoreLicensesButtonFocusRef.current = false;
    onClose();
  }, [onClose]);

  const handleLicensesButtonRef = useCallback(
    (node: HTMLButtonElement | null) => {
      if (node && restoreLicensesButtonFocusRef.current) {
        restoreLicensesButtonFocusRef.current = false;
        node.focus();
      }
    },
    [],
  );

  const handleOpenLicenses = useCallback(() => {
    setLicenseQuery("");
    setView("licenses");
  }, []);

  const handleBackToAbout = useCallback(() => {
    restoreLicensesButtonFocusRef.current = true;
    setLicenseQuery("");
    setView("about");
  }, []);

  const filteredLicenses = useMemo(() => {
    const query = licenseQuery.trim().toLocaleLowerCase();
    if (!query) return licenses;
    return licenses.filter(
      ({ name, license }) =>
        name.toLocaleLowerCase().includes(query) ||
        license.toLocaleLowerCase().includes(query),
    );
  }, [licenseQuery]);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) handleClose();
      }}
    >
      <DialogContent
        className={
          view === "licenses"
            ? "flex h-[min(80dvh,40rem)] max-w-lg flex-col overflow-hidden"
            : "max-w-sm"
        }
      >
        <StepTransition
          transitionKey={view}
          direction={view === "licenses" ? 1 : -1}
          className={
            view === "licenses"
              ? "flex min-h-0 flex-1 flex-col gap-4"
              : "grid gap-4"
          }
        >
          {view === "about" ? (
            <>
              <DialogHeader>
                <DialogTitle>{t("about.title")}</DialogTitle>
              </DialogHeader>

              <div className="flex flex-col items-center gap-3 py-4">
                <div className="text-center">
                  <p className="text-lg font-semibold">Salamander Browser</p>
                  {systemInfo && (
                    <>
                      <p className="text-sm text-muted-foreground">
                        {t("about.version", {
                          version: systemInfo.app_version,
                        })}
                        {systemInfo.portable && (
                          <span className="ml-1.5 rounded border border-border bg-muted px-1 py-px text-[10px] align-middle">
                            {t("about.portableBadge")}
                          </span>
                        )}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {systemInfo.os} {systemInfo.arch}
                      </p>
                    </>
                  )}
                </div>

                <p className="text-center text-xs text-muted-foreground">
                  {t("about.licenseNotice")}
                </p>

                <div className="flex flex-wrap justify-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      void openUrl(
                        "https://github.com/mb-technical-solutions/salamanderbrowser",
                      )
                    }
                  >
                    {t("about.github")}
                  </Button>
                  <Button
                    ref={handleLicensesButtonRef}
                    variant="outline"
                    size="sm"
                    onClick={handleOpenLicenses}
                  >
                    {t("about.licenses")}
                  </Button>
                </div>
              </div>

              <div className="flex justify-end">
                <RippleButton variant="outline" onClick={handleClose}>
                  {t("common.buttons.close")}
                </RippleButton>
              </div>
            </>
          ) : (
            <>
              <DialogHeader className="flex-row items-center gap-2">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="-ml-2 size-8"
                  aria-label={t("common.buttons.back")}
                  onClick={handleBackToAbout}
                >
                  <LuArrowLeft aria-hidden="true" />
                </Button>
                <DialogTitle>{t("about.licenses")}</DialogTitle>
              </DialogHeader>

              <div className="relative shrink-0">
                <LuSearch
                  aria-hidden="true"
                  className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
                />
                <Input
                  autoFocus
                  type="search"
                  value={licenseQuery}
                  onChange={(event) => {
                    setLicenseQuery(event.target.value);
                  }}
                  aria-label={t("about.searchLicenses")}
                  placeholder={t("about.searchLicenses")}
                  className="pl-9"
                />
              </div>

              <FadingScrollArea
                role="region"
                tabIndex={0}
                aria-label={t("about.licenses")}
                className="-mx-2 min-h-0 flex-1 rounded-md px-2 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
              >
                {filteredLicenses.length > 0 ? (
                  <ul>
                    {filteredLicenses.map(({ name, license }) => (
                      <li
                        key={`${name}-${license}`}
                        className="grid grid-cols-[minmax(0,1fr)_minmax(7rem,45%)] items-baseline gap-3 border-b border-border/60 py-2.5 last:border-b-0"
                      >
                        {name === "Xray-core" ? (
                          <button
                            type="button"
                            onClick={() => void openUrl(xraySource.sourceUrl)}
                            aria-label={t("about.openSource", { name })}
                            title={t("about.openSource", { name })}
                            className="flex min-w-0 items-center gap-1.5 rounded-sm text-left text-sm font-medium text-foreground underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 active:scale-[0.98] motion-reduce:transform-none"
                          >
                            <span className="truncate">{name}</span>
                            <LuExternalLink
                              aria-hidden="true"
                              className="size-3.5 shrink-0 text-muted-foreground"
                            />
                          </button>
                        ) : (
                          <span
                            className="truncate text-sm font-medium text-foreground"
                            title={name}
                          >
                            {name}
                          </span>
                        )}
                        <code className="break-words text-right text-xs text-muted-foreground">
                          {license}
                        </code>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p
                    role="status"
                    className="py-12 text-center text-sm text-muted-foreground"
                  >
                    {t("about.noMatchingLicenses")}
                  </p>
                )}
              </FadingScrollArea>
            </>
          )}
        </StepTransition>
      </DialogContent>
    </Dialog>
  );
}
