"use client";

import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal } from "react-icons/go";
import { LuInfo, LuKeyboard, LuPuzzle, LuUser, LuUsers } from "react-icons/lu";
import { MOTION_SPRING_POSITION } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

export type AppPage =
  | "profiles"
  | "proxies"
  | "extensions"
  | "groups"
  | "vpns"
  | "settings"
  | "import"
  | "shortcuts";

interface RailNavProps {
  currentPage: AppPage;
  onNavigate: (page: AppPage) => void;
  onOpenAbout: () => void;
}

/** Shared-element indicator that slides between the active rail items. */
function ActiveIndicator() {
  return (
    <motion.span
      aria-hidden="true"
      layoutId="rail-indicator"
      transition={MOTION_SPRING_POSITION}
      className="absolute inset-y-1.5 left-[-7px] w-[2px] rounded-full bg-foreground"
    />
  );
}

interface RailItem {
  page: AppPage;
  Icon: React.ComponentType<{ className?: string }>;
  labelKey: string;
}

const TOP_ITEMS: RailItem[] = [
  { page: "profiles", Icon: LuUser, labelKey: "rail.profiles" },
  { page: "proxies", Icon: FiWifi, labelKey: "rail.network" },
  { page: "extensions", Icon: LuPuzzle, labelKey: "rail.extensions" },
  { page: "groups", Icon: LuUsers, labelKey: "rail.groups" },
];

interface MoreMenuItem {
  page: AppPage;
  Icon: React.ComponentType<{ className?: string }>;
  labelKey: string;
  hintKey: string;
}

const MORE_ITEMS: MoreMenuItem[] = [
  {
    page: "import",
    Icon: FaDownload,
    labelKey: "rail.more.importProfile",
    hintKey: "rail.more.importProfileHint",
  },
  {
    page: "shortcuts",
    Icon: LuKeyboard,
    labelKey: "rail.more.keyboardShortcuts",
    hintKey: "rail.more.keyboardShortcutsHint",
  },
];

export function RailNav({
  currentPage,
  onNavigate,
  onOpenAbout,
}: RailNavProps) {
  const { t } = useTranslation();
  const [moreOpen, setMoreOpen] = useState(false);

  useEffect(() => {
    if (!moreOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMoreOpen(false);
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [moreOpen]);

  return (
    <nav className="relative flex w-10 shrink-0 flex-col items-center gap-1 border-r border-border bg-background py-2">
      <div className="flex min-h-0 w-full scrollbar-none flex-col items-center gap-1 overflow-y-auto [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden">
        {TOP_ITEMS.map(({ page, Icon, labelKey }) => {
          const active = currentPage === page;
          return (
            <Tooltip key={page} delayDuration={300}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => {
                    onNavigate(page);
                  }}
                  aria-label={t(labelKey)}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "relative grid size-7 shrink-0 cursor-pointer place-items-center rounded-md transition-colors duration-100",
                    active
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                >
                  {active && <ActiveIndicator />}
                  <Icon className="size-3.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{t(labelKey)}</TooltipContent>
            </Tooltip>
          );
        })}
      </div>

      <div className="flex-1" />

      <Tooltip delayDuration={300}>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={() => {
              setMoreOpen((v) => !v);
            }}
            aria-label={t("rail.more.label")}
            aria-expanded={moreOpen}
            className={cn(
              "grid size-7 shrink-0 cursor-pointer place-items-center rounded-md transition-colors duration-100",
              moreOpen
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
            )}
          >
            <GoKebabHorizontal className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{t("rail.more.label")}</TooltipContent>
      </Tooltip>

      <Tooltip delayDuration={300}>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={() => {
              onNavigate("settings");
            }}
            aria-label={t("rail.settings")}
            aria-current={currentPage === "settings" ? "page" : undefined}
            className={cn(
              "relative grid size-7 shrink-0 cursor-pointer place-items-center rounded-md transition-colors duration-100",
              currentPage === "settings"
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
            )}
          >
            {currentPage === "settings" && <ActiveIndicator />}
            <GoGear className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{t("rail.settings")}</TooltipContent>
      </Tooltip>

      {moreOpen && (
        <>
          <button
            type="button"
            aria-label={t("rail.more.closeAriaLabel")}
            className="fixed inset-0 z-30 cursor-default bg-transparent"
            onClick={() => {
              setMoreOpen(false);
            }}
          />
          <div
            role="menu"
            aria-label={t("rail.more.label")}
            className="surface-material-card absolute bottom-14 left-11 z-40 w-56 animate-in rounded-lg border border-border p-1 shadow-2xl duration-100 fade-in-0 slide-in-from-bottom-1"
          >
            {MORE_ITEMS.map(({ page, Icon, labelKey, hintKey }) => (
              <button
                key={page}
                type="button"
                role="menuitem"
                onClick={() => {
                  setMoreOpen(false);
                  onNavigate(page);
                }}
                className="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors duration-100 hover:bg-accent hover:text-accent-foreground"
              >
                <span className="grid size-5 shrink-0 place-items-center rounded bg-muted text-muted-foreground">
                  <Icon className="size-3" />
                </span>
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-xs font-medium text-foreground">
                    {t(labelKey)}
                  </span>
                  <span className="truncate text-[10px] text-muted-foreground">
                    {t(hintKey)}
                  </span>
                </span>
              </button>
            ))}
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMoreOpen(false);
                onOpenAbout();
              }}
              className="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors duration-100 hover:bg-accent hover:text-accent-foreground"
            >
              <span className="grid size-5 shrink-0 place-items-center rounded bg-muted text-muted-foreground">
                <LuInfo className="size-3" />
              </span>
              <span className="flex min-w-0 flex-col">
                <span className="truncate text-xs font-medium text-foreground">
                  {t("rail.more.about")}
                </span>
                <span className="truncate text-[10px] text-muted-foreground">
                  {t("rail.more.aboutHint")}
                </span>
              </span>
            </button>
          </div>
        </>
      )}
    </nav>
  );
}
