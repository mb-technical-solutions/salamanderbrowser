import type { TFunction } from "i18next";

/**
 * Backend error codes returned from Rust Tauri commands.
 * Keep this list in sync with the codes used in `src-tauri/src`.
 */
export type BackendErrorCode =
  | "INCORRECT_PASSWORD"
  | "LOCKED_OUT"
  | "PROFILE_NOT_FOUND"
  | "PROFILE_NOT_PROTECTED"
  | "PROFILE_ALREADY_PROTECTED"
  | "PROFILE_RUNNING"
  | "INVALID_PROFILE_ID"
  | "PROFILE_MISSING_SALT"
  | "PROFILE_LOCKED"
  | "PASSWORD_TOO_SHORT"
  | "COOKIE_DB_LOCKED"
  | "COOKIE_DB_UNAVAILABLE"
  | "COOKIE_IMPORT_BROWSER_RUNNING"
  | "COOKIE_IMPORT_PROFILE_PROTECTED"
  | "COOKIE_IMPORT_NO_COOKIES"
  | "PROXY_NOT_FOUND"
  | "GROUP_NOT_FOUND"
  | "GROUP_ALREADY_EXISTS"
  | "NAME_CANNOT_BE_EMPTY"
  | "VPN_NOT_FOUND"
  | "EXTENSION_NOT_FOUND"
  | "EXTENSION_GROUP_NOT_FOUND"
  | "EXTENSION_UNSUPPORTED_FILE_TYPE"
  | "EXTENSION_DIR_NOT_FOUND"
  | "EXTENSION_NOT_A_DIRECTORY"
  | "EXTENSION_MANIFEST_MISSING"
  | "EXTENSION_MANIFEST_INVALID"
  | "EXTENSION_DIR_TOO_LARGE"
  | "EXTENSION_PATH_HAS_COMMA"
  | "EXTENSION_LINK_REQUIRES_DIRECTORY"
  | "PROXY_NOT_WORKING"
  | "VPN_NOT_WORKING"
  | "PROXY_SIDECAR_VERSION_MISMATCH"
  | "UPDATE_CHECKSUMS_UNAVAILABLE"
  | "UPDATE_CHECKSUM_MISMATCH"
  | "BROWSER_CHECKSUM_UNAVAILABLE"
  | "BROWSER_CHECKSUM_MISMATCH"
  | "UPDATE_PROFILES_RUNNING"
  | "UPDATE_PREPARATION_FAILED"
  | "PROFILE_NAME_EXISTS"
  | "IMPORT_SOURCE_NOT_FOUND"
  | "CAMOUFOX_IMPORT_INVALID_FOLDER"
  | "CAMOUFOX_IMPORT_COPY_FAILED"
  | "BROWSER_NOT_DOWNLOADED"
  | "ARCHIVE_EXTRACTION_FAILED"
  | "UNSUPPORTED_ARCHIVE_FORMAT"
  | "PROXY_AND_VPN_MUTUALLY_EXCLUSIVE"
  | "FINGERPRINT_MATCH_FAILED"
  | "INVALID_DNS_RULES_JSON"
  | "UNSUPPORTED_DNS_RULES_FORMAT"
  | "DNS_RULES_SAVE_FAILED"
  | "DNS_RULES_EXPORT_FAILED"
  | "VLESS_CONFIG_INVALID"
  | "VLESS_SUBSCRIPTION_FETCH_FAILED"
  | "XRAY_UNAVAILABLE"
  | "XRAY_UNSUPPORTED_OS"
  | "XRAY_START_FAILED"
  | "FINGERPRINT_EXIT_MISMATCH"
  | "LAUNCH_CONSENT_EXPIRED"
  | "VPN_WORKER_START_FAILED"
  | "EXIT_PROBE_FAILED"
  | "ARCHIVE_PASSWORD_REQUIRED"
  | "ARCHIVE_WRONG_PASSWORD"
  | "ARCHIVE_CORRUPT"
  | "ARCHIVE_INVALID_PATH"
  | "ARCHIVE_TOO_LARGE"
  | "ARCHIVE_UNSUPPORTED_SYMLINK"
  | "ARCHIVE_UNSUPPORTED_VERSION"
  | "EXPORT_WRITE_FAILED"
  | "IMPORT_COPY_FAILED"
  | "INTERNAL_ERROR";

export interface BackendError {
  code: BackendErrorCode;
  params?: Record<string, string>;
}

/**
 * Try to parse a backend error string as a structured `{code, params}` payload.
 * Returns null if the string isn't structured (e.g. raw error from a command
 * that doesn't yet emit codes — caller should fall back to showing the raw text).
 */
export function parseBackendError(err: unknown): BackendError | null {
  const message = err instanceof Error ? err.message : String(err);
  if (!message.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(message);
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.code === "string"
    ) {
      return parsed as BackendError;
    }
  } catch {
    // not JSON
  }
  return null;
}

/**
 * Translate a backend error to a localized string. Falls back to the raw
 * message if the error isn't a structured backend error.
 */
export function translateBackendError(t: TFunction, err: unknown): string {
  const parsed = parseBackendError(err);
  if (!parsed) {
    return err instanceof Error ? err.message : String(err);
  }
  switch (parsed.code) {
    case "PROFILE_NOT_FOUND":
      return t("backendErrors.profileNotFound");
    case "INCORRECT_PASSWORD":
      return t("backendErrors.incorrectPassword");
    case "LOCKED_OUT": {
      const seconds = Number.parseInt(parsed.params?.seconds ?? "0", 10);
      return t("backendErrors.lockedOut", {
        duration: formatLockoutDuration(t, seconds),
      });
    }
    case "PROFILE_NOT_PROTECTED":
      return t("backendErrors.profileNotProtected");
    case "PROFILE_ALREADY_PROTECTED":
      return t("backendErrors.profileAlreadyProtected");
    case "PROFILE_RUNNING":
      return t("backendErrors.profileRunning");
    case "PROFILE_MISSING_SALT":
      return t("backendErrors.profileMissingSalt");
    case "PROFILE_LOCKED":
      return t("backendErrors.profileLocked");
    case "INVALID_PROFILE_ID":
      return t("backendErrors.invalidProfileId");
    case "PASSWORD_TOO_SHORT": {
      const min = Number.parseInt(parsed.params?.min ?? "8", 10);
      return t("backendErrors.passwordTooShort", { min });
    }
    case "COOKIE_DB_LOCKED":
      return t("backendErrors.cookieDbLocked");
    case "COOKIE_DB_UNAVAILABLE":
      return t("backendErrors.cookieDbUnavailable");
    case "COOKIE_IMPORT_BROWSER_RUNNING":
      return t("backendErrors.cookieImportBrowserRunning");
    case "COOKIE_IMPORT_PROFILE_PROTECTED":
      return t("backendErrors.cookieImportProfileProtected");
    case "COOKIE_IMPORT_NO_COOKIES":
      return t("backendErrors.cookieImportNoCookies");
    case "PROXY_NOT_FOUND":
      return t("backendErrors.proxyNotFound");
    case "GROUP_NOT_FOUND":
      return t("backendErrors.groupNotFound");
    case "GROUP_ALREADY_EXISTS":
      return t("backendErrors.groupAlreadyExists");
    case "NAME_CANNOT_BE_EMPTY":
      return t("backendErrors.nameCannotBeEmpty");
    case "VPN_NOT_FOUND":
      return t("backendErrors.vpnNotFound");
    case "EXTENSION_NOT_FOUND":
      return t("backendErrors.extensionNotFound");
    case "EXTENSION_GROUP_NOT_FOUND":
      return t("backendErrors.extensionGroupNotFound");
    case "EXTENSION_UNSUPPORTED_FILE_TYPE":
      return t("backendErrors.extensionUnsupportedFileType");
    case "EXTENSION_DIR_NOT_FOUND":
      return t("backendErrors.extensionDirNotFound");
    case "EXTENSION_NOT_A_DIRECTORY":
      return t("backendErrors.extensionNotADirectory");
    case "EXTENSION_MANIFEST_MISSING":
      return t("backendErrors.extensionManifestMissing");
    case "EXTENSION_MANIFEST_INVALID":
      return t("backendErrors.extensionManifestInvalid");
    case "EXTENSION_DIR_TOO_LARGE":
      return t("backendErrors.extensionDirTooLarge");
    case "EXTENSION_PATH_HAS_COMMA":
      return t("backendErrors.extensionPathHasComma");
    case "EXTENSION_LINK_REQUIRES_DIRECTORY":
      return t("backendErrors.extensionLinkRequiresDirectory");
    case "PROXY_NOT_WORKING":
      return t("backendErrors.proxyNotWorking");
    case "VPN_NOT_WORKING":
      return t("backendErrors.vpnNotWorking");
    case "PROXY_SIDECAR_VERSION_MISMATCH":
      return t("backendErrors.proxySidecarVersionMismatch");
    case "UPDATE_CHECKSUMS_UNAVAILABLE":
      return t("backendErrors.updateChecksumsUnavailable", {
        version: parsed.params?.version ?? "",
      });
    case "UPDATE_CHECKSUM_MISMATCH":
      return t("backendErrors.updateChecksumMismatch", {
        file: parsed.params?.file ?? "",
      });
    case "BROWSER_CHECKSUM_UNAVAILABLE":
      return t("backendErrors.browserChecksumUnavailable", {
        browser: parsed.params?.browser ?? "",
        version: parsed.params?.version ?? "",
      });
    case "BROWSER_CHECKSUM_MISMATCH":
      return t("backendErrors.browserChecksumMismatch", {
        browser: parsed.params?.browser ?? "",
        version: parsed.params?.version ?? "",
      });
    case "UPDATE_PROFILES_RUNNING":
      return t("backendErrors.updateProfilesRunning");
    case "UPDATE_PREPARATION_FAILED":
      return t("backendErrors.updatePreparationFailed");
    case "PROFILE_NAME_EXISTS":
      return t("backendErrors.profileNameExists", {
        name: parsed.params?.name ?? "",
      });
    case "IMPORT_SOURCE_NOT_FOUND":
      return t("backendErrors.importSourceNotFound");
    case "CAMOUFOX_IMPORT_INVALID_FOLDER":
      return t("backendErrors.camoufoxImportInvalidFolder");
    case "CAMOUFOX_IMPORT_COPY_FAILED":
      return t("backendErrors.camoufoxImportCopyFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "BROWSER_NOT_DOWNLOADED":
      return t("backendErrors.browserNotDownloaded", {
        browser: parsed.params?.browser ?? "",
      });
    case "ARCHIVE_EXTRACTION_FAILED":
      return t("backendErrors.archiveExtractionFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "UNSUPPORTED_ARCHIVE_FORMAT":
      return t("backendErrors.unsupportedArchiveFormat");
    case "PROXY_AND_VPN_MUTUALLY_EXCLUSIVE":
      return t("backendErrors.proxyAndVpnMutuallyExclusive");
    case "FINGERPRINT_MATCH_FAILED":
      return t("backendErrors.fingerprintMatchFailed");
    case "INVALID_DNS_RULES_JSON":
      return t("backendErrors.invalidDnsRulesJson");
    case "UNSUPPORTED_DNS_RULES_FORMAT":
      return t("backendErrors.unsupportedDnsRulesFormat", {
        format: parsed.params?.format ?? "",
      });
    case "DNS_RULES_SAVE_FAILED":
      return t("backendErrors.dnsRulesSaveFailed");
    case "DNS_RULES_EXPORT_FAILED":
      return t("backendErrors.dnsRulesExportFailed");
    // Salamander supports a specific, checked set of VLESS shapes (TCP/WS/gRPC/H2
    // transports, over none/TLS/REALITY security), so most rejections mean
    // "your server is a kind we do not support", not "you mistyped". Name the
    // unsupported part instead of implying a typo.
    case "VLESS_CONFIG_INVALID": {
      const reason = parsed.params?.reason;
      const known = [
        "security",
        "flow",
        "transport",
        "encryption",
        "headerType",
        "fingerprint",
        "sni",
        "publicKey",
        "alpn",
        "scheme",
        "parameter",
        "malformed",
      ];
      if (reason && known.includes(reason)) {
        return t(`backendErrors.vlessUnsupported.${reason}`);
      }
      return t("backendErrors.vlessConfigInvalid");
    }
    case "VLESS_SUBSCRIPTION_FETCH_FAILED":
      return t("backendErrors.vlessSubscriptionFetchFailed");
    case "XRAY_UNAVAILABLE":
      return t("backendErrors.xrayUnavailable");
    case "XRAY_UNSUPPORTED_OS":
      return t("backendErrors.xrayUnsupportedOs");
    case "XRAY_START_FAILED":
      return t("backendErrors.xrayStartFailed");
    // The launch gate's block. The dialog renders the mismatch detail from
    // `params` itself; this string is the fallback for anywhere that only has
    // room for one sentence.
    case "FINGERPRINT_EXIT_MISMATCH":
      return t("backendErrors.fingerprintExitMismatch");
    case "LAUNCH_CONSENT_EXPIRED":
      return t("backendErrors.launchConsentExpired");
    case "VPN_WORKER_START_FAILED":
      return t("backendErrors.vpnWorkerStartFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "EXIT_PROBE_FAILED":
      return t("backendErrors.exitProbeFailed");
    case "ARCHIVE_PASSWORD_REQUIRED":
      return t("backendErrors.archivePasswordRequired");
    case "ARCHIVE_WRONG_PASSWORD":
      return t("backendErrors.archiveWrongPassword");
    case "ARCHIVE_CORRUPT":
      return t("backendErrors.archiveCorrupt");
    case "ARCHIVE_INVALID_PATH":
    case "ARCHIVE_TOO_LARGE":
    case "ARCHIVE_UNSUPPORTED_SYMLINK":
      return t("backendErrors.archiveCorrupt");
    case "ARCHIVE_UNSUPPORTED_VERSION":
      return t("backendErrors.archiveUnsupportedVersion");
    case "EXPORT_WRITE_FAILED":
      return t("backendErrors.exportWriteFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "IMPORT_COPY_FAILED":
      return t("backendErrors.importCopyFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "INTERNAL_ERROR":
      return t("backendErrors.internal", {
        detail: parsed.params?.detail ?? "",
      });
    default:
      // The payload parsed as a structured error but carries a code this build
      // does not know: the server can add codes faster than the desktop ships.
      // Returning the raw message here would render the literal JSON to the
      // user, so show a translated line that still names the code for support.
      return t("backendErrors.unknownCode", { code: String(parsed.code) });
  }
}

/**
 * True if the error is a known structured backend error code.
 */
export function isBackendErrorCode(
  err: unknown,
  code: BackendErrorCode,
): boolean {
  return parseBackendError(err)?.code === code;
}

export function formatLockoutDuration(t: TFunction, seconds: number): string {
  if (seconds < 60) return t("backendErrors.lockedOutDuration.seconds", { seconds });
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return t("backendErrors.lockedOutDuration.minutes", { minutes });
  return t("backendErrors.lockedOutDuration.hours", {
    hours: Math.ceil(minutes / 60),
  });
}

export function extractLockoutSeconds(err: unknown): number | null {
  const parsed = parseBackendError(err);
  if (parsed?.code !== "LOCKED_OUT") return null;
  const seconds = Number.parseInt(parsed.params?.seconds ?? "0", 10);
  return Number.isFinite(seconds) && seconds > 0 ? seconds : null;
}
