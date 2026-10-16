export interface ProxySettings {
  proxy_type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
  vless_uri?: string;
}

export interface TableSortingSettings {
  column: string; // "name", "note", "status"
  direction: string; // "asc" or "desc"
}

export interface BrowserProfile {
  id: string; // UUID of the profile
  name: string;
  browser: string;
  version: string;
  proxy_id?: string; // Reference to stored proxy
  vpn_id?: string; // Reference to stored VPN config
  process_id?: number;
  last_launch?: number;
  release_type: string;
  camoufox_config?: CamoufoxConfig; // Camoufox configuration
  group_id?: string; // Reference to profile group
  tags?: string[];
  note?: string; // User note
  window_color?: string; // Per-profile window frame color "#RRGGBB"; auto-derived from the id when unset
  password_protected?: boolean;
  encryption_salt?: string;
  host_os?: string; // OS where profile was created ("macos", "windows", "linux")
  extension_group_id?: string;
  proxy_bypass_rules?: string[];
  /** Profile creation timestamp (epoch seconds, UTC). Undefined for legacy
   * profiles created before this field existed. */
  created_at?: number;
  dns_blocklist?: string;
  /** Browser-specific settings for engines other than Camoufox. */
  browser_config?: Record<string, unknown>;
}

export interface PrelaunchSite {
  name: string;
  url: string;
}

export interface ClearcoteConfig {
  fingerprint_seed?: string;
  fingerprint_platform?: CamoufoxOS;
  preset_tabs?: string[];
}

export type BrowserType = "camoufox" | "clearcote";

export interface Extension {
  id: string;
  name: string;
  file_name: string;
  file_type: string;
  browser_compatibility: string[];
  created_at: number;
  updated_at: number;
  version?: string;
  description?: string;
  author?: string;
  homepage_url?: string;
  /** How the payload was imported: a `.crx`/`.zip` archive, or a folder. */
  source_kind: "archive" | "unpacked";
  /** Absolute folder the extension is loaded from in place. Set means nothing
   * was copied into Salamander, so the extension is machine-local and never syncs. */
  linked_path?: string;
}

export interface ExtensionGroup {
  id: string;
  name: string;
  extension_ids: string[];
  created_at: number;
  updated_at: number;
}

export interface ProxyCheckResult {
  ip: string;
  city?: string;
  country?: string;
  country_code?: string;
  timestamp: number;
  is_valid: boolean;
}

export interface StoredProxy {
  id: string;
  name: string;
  proxy_settings: ProxySettings;
  geo_country?: string;
  geo_region?: string;
  geo_city?: string;
  geo_isp?: string;
}

export interface LocationItem {
  code: string;
  name: string;
}

export interface ProfileGroup {
  id: string;
  name: string;
}

export interface GroupWithCount {
  id: string;
  name: string;
  count: number;
}

export interface BrowserReleaseTypes {
  stable?: string;
}

export interface AppUpdateInfo {
  current_version: string;
  new_version: string;
  release_notes: string;
  download_url: string;
  is_nightly: boolean;
  published_at: string;
  manual_update_required: boolean;
  release_page_url?: string;
  repo_update: boolean;
  /** URL of the release's SHA256SUMS.txt; downloads are verified against it. */
  checksums_url?: string | null;
  /** GitHub-computed digest of the chosen asset ("sha256:<hex>"). */
  asset_digest?: string | null;
}

export interface AppUpdateProgress {
  stage: string; // "downloading", "extracting", "installing", "completed"
  percentage?: number;
  speed?: string; // MB/s
  eta?: string; // estimated time remaining
  message: string;
}

export type CamoufoxOS = "windows" | "macos" | "linux";

/** Camoufox (Firefox-based anti-detect engine) profile configuration. Mirrors
 * `CamoufoxConfig` in `camoufox_manager.rs`. The fingerprint comes from a
 * local Bayesian-network generator, not a launched device. */
export interface CamoufoxConfig {
  proxy?: string;
  screen_max_width?: number;
  screen_max_height?: number;
  screen_min_width?: number;
  screen_min_height?: number;
  geoip?: string | boolean;
  block_images?: boolean;
  block_webrtc?: boolean;
  block_webgl?: boolean;
  fingerprint?: string; // Flat dotted-key JSON config Camoufox reads via CAMOU_CONFIG_N env vars
  randomize_fingerprint_on_launch?: boolean;
  os?: CamoufoxOS; // Camoufox has no mobile fingerprints
  /** URLs opened as tabs every time the profile launches (quick-launch presets). */
  preset_tabs?: string[];
}

// Traffic stats types
export interface BandwidthDataPoint {
  timestamp: number;
  bytes_sent: number;
  bytes_received: number;
}

export interface DomainAccess {
  domain: string;
  request_count: number;
  bytes_sent: number;
  bytes_received: number;
  first_access: number;
  last_access: number;
}

export interface TrafficStats {
  proxy_id: string;
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  bandwidth_history: BandwidthDataPoint[];
  domains: Record<string, DomainAccess>;
  unique_ips: string[];
}

export interface TrafficSnapshot {
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  current_bytes_sent: number;
  current_bytes_received: number;
  recent_bandwidth: BandwidthDataPoint[];
}

export interface FilteredTrafficStats {
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  bandwidth_history: BandwidthDataPoint[];
  period_bytes_sent: number;
  period_bytes_received: number;
  period_requests: number;
  domains: Record<string, DomainAccess>;
  unique_ips: string[];
}

// Cookie copy types
export interface UnifiedCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  expires: number;
  is_secure: boolean;
  is_http_only: boolean;
  same_site: number;
  creation_time: number;
  last_accessed: number;
}

export interface DomainCookies {
  domain: string;
  cookies: UnifiedCookie[];
  cookie_count: number;
}

export interface CookieReadResult {
  profile_id: string;
  browser_type: string;
  domains: DomainCookies[];
  total_count: number;
}

export interface SelectedCookie {
  domain: string;
  name: string;
}

export interface CookieCopyRequest {
  source_profile_id: string;
  target_profile_ids: string[];
  selected_cookies: SelectedCookie[];
}

export interface CookieCopyResult {
  target_profile_id: string;
  cookies_copied: number;
  cookies_replaced: number;
  errors: string[];
}

// Cookie paste types. Unlike the copy types above these are serialized with
// `rename_all = "camelCase"`, so the field names differ from the Rust structs.
export type CookieIssueSeverity = "error" | "warning" | "info";

export interface CookieIssue {
  code: string;
  severity: CookieIssueSeverity;
  source: string | null;
  params: Record<string, string>;
}

export type CookiePasteFormat = "json" | "netscape" | "nameValue";

export type CookieWriteMode = "merge" | "replaceMatchingSites";

/** Carries no `value`: the value is the credential and never leaves Rust. */
export interface PastedCookiePreview {
  name: string;
  domain: string;
  path: string;
  expires: number;
  isSecure: boolean;
  isHttpOnly: boolean;
  sameSite: number;
}

export interface CookieAnalysis {
  format: CookiePasteFormat | null;
  cookies: PastedCookiePreview[];
  issues: CookieIssue[];
  siteRequired: boolean;
  expiredCount: number;
  /** `null` when the store cannot be read, which is not the same as zero. */
  replaceDeleteCount: number | null;
  /** A `{"code":…}` string for `translateBackendError`, or `null` to proceed. */
  blockedBy: string | null;
}

export interface CookiePasteImportResult {
  added: number;
  overwritten: number;
  deleted: number;
  skipped: number;
  issues: CookieIssue[];
}

// Proxy import/export types
export interface ProxyExportData {
  version: string;
  proxies: ExportedProxy[];
  exported_at: string;
  source: string;
}

export interface ExportedProxy {
  name: string;
  type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
}

export interface ProxyImportResult {
  imported_count: number;
  skipped_count: number;
  errors: string[];
  proxies: StoredProxy[];
}

export interface ParsedProxyLine {
  proxy_type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
  vless_uri?: string;
  original_line: string;
}

export type ProxyParseResult =
  | ({ status: "parsed" } & ParsedProxyLine)
  | { status: "ambiguous"; line: string; possible_formats: string[] }
  | { status: "invalid"; line: string; reason: string };

export interface SubscriptionEntry {
  name: string;
  uri: string;
}

export interface SubscriptionSkip {
  preview: string;
  reason: string;
}

export interface SubscriptionImportResult {
  entries: SubscriptionEntry[];
  skipped: SubscriptionSkip[];
}

// VPN types
export type VpnType = "WireGuard";

export interface VpnConfig {
  id: string;
  name: string;
  vpn_type: VpnType;
  config_data: string; // Raw config content (may be empty in list view)
  created_at: number;
  last_used?: number;
}

export interface VpnImportResult {
  success: boolean;
  vpn_id?: string;
  vpn_type?: VpnType;
  name: string;
  error?: string;
}

export interface VpnStatus {
  connected: boolean;
  vpn_id: string;
  connected_at?: number;
  bytes_sent?: number;
  bytes_received?: number;
  last_handshake?: number;
}

/** Result of comparing a proxy's exit node against a profile's fingerprint. */
export interface ConsistencyResult {
  consistent: boolean;
  checked: boolean;
  exit_ip: string | null;
  exit_country_code: string | null;
  exit_timezone: string | null;
  fingerprint_timezone: string | null;
  fingerprint_language: string | null;
  /** Which dimensions disagree: "timezone", "language". */
  mismatches: string[];
}

/**
 * How strongly an extension is believed to be a VPN or proxy tool.
 * "capability" is not such a claim: it means only that the extension holds
 * Chromium's `proxy` permission, which download managers do too.
 */
export type VpnExtensionConfidence = "confirmed" | "likely" | "capability";

/** How much of a profile's extension set could be read. */
export type ExtensionScanState = "scanned" | "partial" | "missing";

/** An extension found in a profile that could change where the browser connects. */
export interface DetectedVpnExtension {
  /** Acknowledgement identity: `salamander:<uuid>` or `crx:<id>`. */
  key: string;
  name: string;
  version: string | null;
  /** "salamander" (managed by Salamander) or "browser" (installed in the profile). */
  source: string;
  confidence: VpnExtensionConfidence;
  /** Holds the `proxy` permission outright, so it can change the proxy today. */
  proxy_control: boolean;
  signals: string[];
}

/** Local-only checks answered before a launch starts any worker. */
export interface PreLaunchChecks {
  vpn_extensions: DetectedVpnExtension[];
  scan_state: ExtensionScanState;
  consistency: ConsistencyResult;
  exit_probe_pending: boolean;
  exit_measurement_unreliable: boolean;
  consent_token: string | null;
}

/**
 * What happened when the user asked Salamander to become the default browser.
 *
 * macOS and Linux let a program make the change itself, so the answer there is
 * always "set". Windows reserves the final choice for its own settings page:
 * the app registers itself, Windows Settings opens, and the user finishes the
 * job. Treating that case as plain success is how the button used to report a
 * change that had not happened.
 */
export type SetDefaultBrowserOutcome =
  | { status: "set" }
  | { status: "awaitingSystemSettings" };
