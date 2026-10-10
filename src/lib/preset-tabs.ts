/** Quick-launch preset tabs offered when creating/editing a Camoufox profile.
 * Applied via `CamoufoxConfig.preset_tabs`, which the backend turns into
 * Firefox's `browser.startup.homepage` prefs so they open automatically on
 * every launch. */
export const PRESET_TAB_OPTIONS: { key: string; url: string }[] = [
  { key: "google", url: "https://www.google.com/" },
  { key: "youtube", url: "https://www.youtube.com/" },
  { key: "instagram", url: "https://www.instagram.com/" },
];
