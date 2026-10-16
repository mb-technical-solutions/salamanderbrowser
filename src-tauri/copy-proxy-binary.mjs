import { execFileSync, execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { downloadXray } from "./download-xray.mjs";

// VPN workers run `salamander-proxy.exe` detached so a tunnel survives the
// window being hidden to tray — which also means a leftover process from a
// prior dev/test run can still be holding the exe open. `cargo build` runs
// this crate's build.rs regardless of which bin target you're building, and
// that script copies the very same sidecar binary, so a locked file fails
// the build with a raw `PermissionDenied` panic that gives no hint it's just
// a stale process. Clear it first, on every local rebuild, not only inside
// the packaged NSIS installer.
function killLingeringSidecars() {
  if (process.platform !== "win32") return;
  try {
    execFileSync("taskkill", ["/F", "/T", "/IM", "salamander-proxy.exe"], {
      stdio: "ignore",
    });
  } catch {
    // Nothing running under that name — fine, this is just a precaution.
  }
}

killLingeringSidecars();

const MANIFEST_DIR = dirname(fileURLToPath(import.meta.url));
const PROFILE =
  process.argv.includes("--release") || process.env.PROFILE === "release"
    ? "release"
    : "debug";

function getTarget() {
  if (process.env.TARGET) return process.env.TARGET;
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

function getHostTarget() {
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

const TARGET = getTarget();
const HOST_TARGET = getHostTarget();
const isWindows = TARGET.includes("windows");

// Determine source directory
let srcDir;
if (TARGET === HOST_TARGET || TARGET === "unknown") {
  srcDir = join(
    MANIFEST_DIR,
    "target",
    PROFILE === "release" ? "release" : "debug",
  );
} else {
  srcDir = join(
    MANIFEST_DIR,
    "target",
    TARGET,
    PROFILE === "release" ? "release" : "debug",
  );
}

const destDir = join(MANIFEST_DIR, "binaries");
mkdirSync(destDir, { recursive: true });

function copyBinary(baseName) {
  const binName = isWindows ? `${baseName}.exe` : baseName;
  const source = join(srcDir, binName);

  let destName = `${baseName}-${TARGET}`;
  if (isWindows) destName += ".exe";
  const dest = join(destDir, destName);

  const buildArgs = ["build", "--bin", baseName];
  if (PROFILE === "release") buildArgs.push("--release");
  if (TARGET !== "unknown" && TARGET !== HOST_TARGET) {
    buildArgs.push("--target", TARGET);
  }
  execFileSync("cargo", buildArgs, {
    cwd: MANIFEST_DIR,
    stdio: "inherit",
  });

  if (!existsSync(source)) {
    console.error(`Error: Failed to build ${baseName} binary`);
    process.exit(1);
  }
  copyFileSync(source, dest);
  console.log(`Built and copied ${binName} to ${dest}`);
}

copyBinary("salamander-proxy");
await downloadXray(TARGET);
