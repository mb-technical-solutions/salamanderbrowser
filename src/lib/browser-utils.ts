/**
 * Browser utility functions
 * Centralized helpers for browser name mapping, icons, etc.
 */

import { FaChrome, FaExclamationTriangle, FaFirefoxBrowser } from "react-icons/fa";
import { getCurrentOS } from "@/lib/platform";

export { getCurrentOS } from "@/lib/platform";

/**
 * Map internal browser names to display names
 */
export function getBrowserDisplayName(browserType: string): string {
  const browserNames: Record<string, string> = {
    camoufox: "Firefox",
    clearcote: "Chrome",
  };

  return browserNames[browserType] || browserType;
}

/**
 * Get the appropriate icon component for a browser type
 * Anti-detect browsers get their base browser icons
 * Other browsers get a warning icon to indicate they're not anti-detect
 */
export function getBrowserIcon(browserType: string) {
  switch (browserType) {
    case "camoufox":
      return FaFirefoxBrowser;
    case "clearcote":
      return FaChrome;
    default:
      return FaExclamationTriangle;
  }
}

/** The backend currently implements cookie operations for Camoufox only. */
export function supportsCookieBackend(browserType: string): boolean {
  return browserType === "camoufox";
}

export function getProfileIcon(profile: { browser: string }) {
  return getBrowserIcon(profile.browser);
}

export function isCrossOsProfile(profile: {
  host_os?: string;
  camoufox_config?: { os?: string };
}): boolean {
  const profileOs = profile.host_os || profile.camoufox_config?.os;
  if (!profileOs) return false;
  return profileOs !== getCurrentOS();
}

export function getOSDisplayName(os: string): string {
  switch (os) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return os;
  }
}
