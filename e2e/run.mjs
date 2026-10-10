#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import {
  copyFileSync,
  createReadStream,
  createWriteStream,
  existsSync,
  readFileSync,
  statSync,
} from "node:fs";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { createSafeDiagnostics } from "./lib/diagnostics.mjs";

const dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(dirname, "..");
const isWindows = process.platform === "win32";
const executableSuffix = isWindows ? ".exe" : "";
const appManifestDir = path.join(projectRoot, "e2e", "app");
const appBinary = path.join(
  appManifestDir,
  "target",
  "debug",
  `salamanderbrowser-e2e${executableSuffix}`,
);
const driverRoot = path.join(projectRoot, "e2e", ".driver");
const driverBinary = path.join(
  driverRoot,
  "bin",
  `tauri-wd${executableSuffix}`,
);
const appManifest = path.join(appManifestDir, "Cargo.toml");
const appLockfile = path.join(appManifestDir, "Cargo.lock");
const salamanderLockfile = path.join(projectRoot, "src-tauri", "Cargo.lock");

const suiteFiles = {
  smoke: ["diagnostics.test.mjs", "smoke.test.mjs", "coverage.test.mjs"],
  ui: ["ui.test.mjs"],
  entities: ["entities.test.mjs"],
  network: ["network.test.mjs"],
  integrations: ["integrations.test.mjs"],
  full: [
    "diagnostics.test.mjs",
    "coverage.test.mjs",
    "smoke.test.mjs",
    "ui.test.mjs",
    "entities.test.mjs",
    "network.test.mjs",
    "integrations.test.mjs",
  ],
};

function parseArgs(argv) {
  const options = {
    suite: "full",
    build: true,
    keep: process.env.SALAMANDER_E2E_KEEP_ARTIFACTS === "1",
    verbose: process.env.SALAMANDER_E2E_VERBOSE === "1",
  };
  for (const arg of argv) {
    if (arg.startsWith("--suite=")) {
      options.suite = arg.slice("--suite=".length);
    } else if (arg === "--no-build") {
      options.build = false;
    } else if (arg === "--keep") {
      options.keep = true;
    } else if (arg === "--verbose") {
      options.verbose = true;
    } else {
      throw new Error(`Unknown E2E option: ${arg}`);
    }
  }
  if (!suiteFiles[options.suite]) {
    throw new Error(
      `Unknown suite ${options.suite}; expected ${Object.keys(suiteFiles).join(", ")}`,
    );
  }
  return options;
}

function log(message) {
  process.stdout.write(`[salamander-e2e] ${message}\n`);
}

function run(command, args, cwd, env = process.env) {
  log(`${command} ${args.join(" ")}`);
  // Windows resolves package-manager shims (pnpm, npm, ...) to a `.CMD`/`.PS1`
  // file that spawnSync cannot exec directly without a shell — every other
  // platform runs the binary as-is, so this only changes behavior on win32.
  // With shell:true, cmd.exe splits an unquoted command on whitespace, so an
  // absolute path like `C:\Program Files\nodejs\node.exe` must be quoted or
  // it is parsed as `C:\Program` with `Files\nodejs\node.exe` as its first
  // argument.
  const resolvedCommand =
    isWindows && command.includes(" ") ? `"${command}"` : command;
  const result = spawnSync(resolvedCommand, args, {
    cwd,
    env,
    stdio: "inherit",
    shell: isWindows,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

async function waitForUrl(url, timeoutMs, processRecord) {
  const started = Date.now();
  let lastError;
  while (Date.now() - started < timeoutMs) {
    if (processRecord?.process.exitCode !== null) {
      throw new Error(
        `${processRecord.name} exited early with ${processRecord.process.exitCode}; see ${processRecord.logPath}`,
      );
    }
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) {
        return;
      }
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError}`);
}

function startProcess(name, command, args, { cwd, env, runRoot, verbose }) {
  const logPath = path.join(runRoot, "logs", `${name}.log`);
  const stream = createWriteStream(logPath, { flags: "a" });
  const child = spawn(command, args, {
    cwd,
    env,
    detached: !isWindows,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.pipe(stream, { end: false });
  child.stderr.pipe(stream, { end: false });
  if (verbose) {
    child.stdout.on("data", (chunk) =>
      process.stdout.write(`[${name}] ${chunk}`),
    );
    child.stderr.on("data", (chunk) =>
      process.stderr.write(`[${name}] ${chunk}`),
    );
  }
  child.on("error", (error) => {
    process.stderr.write(`[salamander-e2e] ${name} process error: ${error}\n`);
  });
  return { name, process: child, stream, logPath };
}

async function stopProcess(record) {
  if (!record || record.process.exitCode !== null) {
    record?.stream.end();
    return;
  }
  if (isWindows) {
    spawnSync("taskkill", ["/PID", String(record.process.pid), "/T", "/F"], {
      stdio: "ignore",
    });
  } else {
    try {
      process.kill(-record.process.pid, "SIGTERM");
    } catch {
      // The process group may already be gone.
    }
  }
  await Promise.race([
    new Promise((resolve) => record.process.once("exit", resolve)),
    new Promise((resolve) => setTimeout(resolve, 5_000)),
  ]);
  if (record.process.exitCode === null && !isWindows) {
    try {
      process.kill(-record.process.pid, "SIGKILL");
    } catch {
      // The process group may already be gone.
    }
  }
  record.stream.end();
}

function unquoteEnvValue(value) {
  const trimmed = value.trim();
  const quote = trimmed[0];
  if ((quote === '"' || quote === "'") && trimmed.at(-1) === quote) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

async function loadLocalValues(names) {
  const values = Object.fromEntries(
    names
      .filter((name) => process.env[name])
      .map((name) => [name, process.env[name]]),
  );
  for (const file of [path.join(projectRoot, ".env")]) {
    try {
      const content = await readFile(file, "utf8");
      for (const name of names) {
        if (values[name]) continue;
        const escapedName = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const match = content.match(
          new RegExp(
            `^\\s*(?:export\\s+)?${escapedName}\\s*=\\s*(.+?)\\s*$`,
            "m",
          ),
        );
        if (match) {
          values[name] = unquoteEnvValue(match[1]);
        }
      }
    } catch {
      // Individual suites validate the secrets they require.
    }
  }
  return values;
}

function pinnedDriverVersion() {
  const manifest = readFileSync(appManifest, "utf8");
  const match = manifest.match(/^tauri-wd\s*=\s*"=([^"]+)"$/m);
  if (!match) {
    throw new Error(
      'e2e/app/Cargo.toml must pin tauri-wd to an exact version, e.g. tauri-wd = "=0.1.11"',
    );
  }
  return match[1];
}

function installedDriverVersion() {
  if (!existsSync(driverBinary)) {
    return null;
  }
  const result = spawnSync(driverBinary, ["--version"], { encoding: "utf8" });
  if (result.status !== 0) {
    return null;
  }
  return result.stdout.trim().split(/\s+/).pop() ?? null;
}

function ensureDriver() {
  const version = pinnedDriverVersion();
  if (installedDriverVersion() === version) {
    log(`tauri-wd ${version} already installed at ${driverBinary}`);
    return;
  }
  run(
    "cargo",
    [
      "install",
      "tauri-wd",
      "--version",
      version,
      "--locked",
      "--debug",
      "--force",
      "--root",
      driverRoot,
    ],
    projectRoot,
  );
}

// The harness links the Salamander crate, so it has to resolve the same versions
// Salamander itself ships. Seeding the harness lockfile from src-tauri/Cargo.lock
// keeps the two in step whenever a dependency or the app version moves; cargo
// fills in the harness-only packages on top. It is generated, never hand-edited.
function syncHarnessLockfile() {
  if (
    existsSync(appLockfile) &&
    statSync(appLockfile).mtimeMs >= statSync(salamanderLockfile).mtimeMs
  ) {
    return;
  }
  copyFileSync(salamanderLockfile, appLockfile);
  log("seeded e2e/app/Cargo.lock from src-tauri/Cargo.lock");
}

function buildAll() {
  run("pnpm", ["build"], projectRoot);
  run("pnpm", ["copy-proxy-binary"], projectRoot);
  run(process.execPath, ["src-tauri/download-xray.mjs"], projectRoot);
  syncHarnessLockfile();
  run("cargo", ["build", "--manifest-path", "e2e/app/Cargo.toml"], projectRoot);
  ensureDriver();
}

function startFixtureServer(geoIpFixture) {
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    if (url.pathname === "/health") {
      response.writeHead(200, { "content-type": "text/plain" });
      response.end("ok");
      return;
    }
    if (url.pathname === "/api/echo") {
      const chunks = [];
      request.on("data", (chunk) => chunks.push(chunk));
      request.on("end", () => {
        response.writeHead(200, {
          "content-type": "application/json",
          "set-cookie": "salamander_e2e=browser-ok; Path=/; SameSite=Lax",
        });
        response.end(
          JSON.stringify({
            method: request.method,
            body: Buffer.concat(chunks).toString("utf8"),
            userAgent: request.headers["user-agent"],
          }),
        );
      });
      return;
    }
    if (url.pathname.startsWith("/dns/")) {
      response.writeHead(200, {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "no-store",
      });
      response.end("ads.e2e.invalid\ntracker.e2e.invalid\n");
      return;
    }
    if (url.pathname === "/geoip.mmdb" && geoIpFixture) {
      response.writeHead(200, {
        "content-type": "application/octet-stream",
        "content-length": String(statSync(geoIpFixture).size),
      });
      createReadStream(geoIpFixture).pipe(response);
      return;
    }
    response.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    });
    response.end(`<!doctype html>
      <html>
        <head><title>Salamander E2E Browser Fixture</title></head>
        <body>
          <h1 id="fixture-title">Salamander E2E Browser Fixture</h1>
          <p id="path">${url.pathname}</p>
          <button id="fixture-button" onclick="this.dataset.clicked='yes'; this.textContent='Clicked'">Click fixture</button>
          <script>window.__fixtureReady = true;</script>
        </body>
      </html>`);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

async function ensureGeoIpFixture() {
  if (process.env.SALAMANDER_E2E_GEOIP_FIXTURE) {
    const fixture = path.resolve(process.env.SALAMANDER_E2E_GEOIP_FIXTURE);
    if (!existsSync(fixture)) {
      throw new Error(`SALAMANDER_E2E_GEOIP_FIXTURE does not exist: ${fixture}`);
    }
    return fixture;
  }
  const toolsDir = path.join(os.tmpdir(), "salamander-e2e-tools");
  const fixture = path.join(toolsDir, "GeoLite2-City.mmdb");
  await mkdir(toolsDir, { recursive: true });
  if (existsSync(fixture)) return fixture;

  log("Downloading GeoLite City E2E dependency");
  const releases = await fetch(
    "https://api.github.com/repos/P3TERX/GeoLite.mmdb/releases",
    {
      headers: { "user-agent": "salamander-browser-e2e" },
      signal: AbortSignal.timeout(30_000),
    },
  ).then((response) => {
    if (!response.ok) {
      throw new Error(
        `GeoLite release lookup failed with HTTP ${response.status}`,
      );
    }
    return response.json();
  });
  const url = releases
    .flatMap((release) => release.assets ?? [])
    .find((asset) => asset.name.endsWith("-City.mmdb"))?.browser_download_url;
  if (!url) throw new Error("No GeoLite City MMDB asset was found");
  const temporary = `${fixture}.${process.pid}.tmp`;
  await download(url, temporary);
  await rename(temporary, fixture);
  return fixture;
}

async function download(url, destination) {
  const transport = url.startsWith("https:") ? https : http;
  await new Promise((resolve, reject) => {
    transport
      .get(url, (response) => {
        if ([301, 302, 307, 308].includes(response.statusCode)) {
          response.resume();
          download(
            new URL(response.headers.location, url).href,
            destination,
          ).then(resolve, reject);
          return;
        }
        if (response.statusCode !== 200) {
          response.resume();
          reject(
            new Error(`Failed to download ${url}: HTTP ${response.statusCode}`),
          );
          return;
        }
        const output = createWriteStream(destination, { mode: 0o755 });
        pipeline(response, output).then(resolve, reject);
      })
      .on("error", reject);
  });
}

function runDocker(args, { allowFailure = false } = {}) {
  const result = spawnSync("docker", args, {
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 10 * 1024 * 1024,
  });
  if (!allowFailure && (result.error || result.status !== 0)) {
    throw new Error(
      `docker ${args[0]} failed: ${result.error?.message ?? result.stderr?.trim() ?? `exit ${result.status}`}`,
    );
  }
  return result;
}

function dockerAvailable() {
  const result = runDocker(["version"], { allowFailure: true });
  return !result.error && result.status === 0;
}

function hostRustTarget() {
  const result = spawnSync("rustc", ["-vV"], {
    encoding: "utf8",
    timeout: 10_000,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      `Could not determine the host Rust target: ${result.error?.message ?? result.stderr?.trim() ?? `exit ${result.status}`}`,
    );
  }
  const target = result.stdout.match(/^host:\s*(.+)$/m)?.[1]?.trim();
  if (!target) {
    throw new Error("rustc -vV did not report a host target");
  }
  return target;
}

function xrayBinaryPath() {
  const target = hostRustTarget();
  return path.join(
    projectRoot,
    "src-tauri",
    "binaries",
    `xray-${target}${target.includes("windows") ? ".exe" : ""}`,
  );
}

async function waitForPort(port, timeoutMs, processRecord) {
  const started = Date.now();
  let lastError;
  while (Date.now() - started < timeoutMs) {
    if (processRecord?.process.exitCode !== null) {
      throw new Error(
        `${processRecord.name} exited early with ${processRecord.process.exitCode}; see ${processRecord.logPath}`,
      );
    }
    try {
      await new Promise((resolve, reject) => {
        const socket = net.createConnection({ host: "127.0.0.1", port });
        socket.setTimeout(1_000);
        socket.once("connect", () => {
          socket.destroy();
          resolve();
        });
        socket.once("timeout", () => {
          socket.destroy();
          reject(new Error("connection timed out"));
        });
        socket.once("error", reject);
      });
      return;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(
    `Timed out waiting for 127.0.0.1:${port}: ${lastError?.message ?? lastError}`,
  );
}

async function startXrayInfrastructure(runRoot, options, records) {
  const executable = xrayBinaryPath();
  if (!existsSync(executable)) {
    throw new Error(
      `The Xray-core E2E binary is missing: ${executable}. Run node src-tauri/download-xray.mjs first.`,
    );
  }

  const keyResult = spawnSync(executable, ["x25519"], {
    encoding: "utf8",
    timeout: 10_000,
  });
  if (keyResult.error || keyResult.status !== 0) {
    throw new Error(
      `Xray-core key generation failed: ${keyResult.error?.message ?? keyResult.stderr?.trim() ?? `exit ${keyResult.status}`}`,
    );
  }
  const privateKey = keyResult.stdout.match(/^PrivateKey:\s*(\S+)\s*$/m)?.[1];
  const publicKey = keyResult.stdout.match(
    /^(?:Password \(PublicKey\)|PublicKey):\s*(\S+)\s*$/m,
  )?.[1];
  if (!privateKey || !publicKey) {
    throw new Error("Xray-core key generation returned an unknown format");
  }

  const port = await freePort();
  const id = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";
  const shortId = "0123456789abcdef";
  const serverName = "www.cloudflare.com";
  const accessLog = path.join(runRoot, "logs", "xray-access.log");
  const configPath = path.join(runRoot, "xray-server.json");
  const config = {
    log: {
      access: accessLog,
      loglevel: "warning",
    },
    inbounds: [
      {
        tag: "vless-reality",
        listen: "127.0.0.1",
        port,
        protocol: "vless",
        settings: {
          clients: [{ id, flow: "xtls-rprx-vision" }],
          decryption: "none",
        },
        streamSettings: {
          network: "raw",
          security: "reality",
          realitySettings: {
            show: false,
            target: `${serverName}:443`,
            xver: 0,
            serverNames: [serverName],
            privateKey,
            shortIds: [shortId],
          },
        },
      },
    ],
    outbounds: [{ tag: "direct", protocol: "freedom" }],
  };
  await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, {
    mode: 0o600,
  });

  const validation = spawnSync(executable, ["run", "-test", "-c", configPath], {
    encoding: "utf8",
    timeout: 15_000,
  });
  if (validation.error || validation.status !== 0) {
    throw new Error(
      `Xray-core server configuration is invalid: ${validation.error?.message ?? validation.stderr?.trim() ?? `exit ${validation.status}`}`,
    );
  }

  const server = startProcess(
    "xray-server",
    executable,
    ["run", "-c", configPath],
    {
      cwd: projectRoot,
      env: process.env,
      runRoot,
      verbose: options.verbose,
    },
  );
  records.push(server);
  await waitForPort(port, 15_000, server);

  const uri = new URL(`vless://${id}@127.0.0.1:${port}`);
  uri.searchParams.set("encryption", "none");
  uri.searchParams.set("flow", "xtls-rprx-vision");
  uri.searchParams.set("security", "reality");
  uri.searchParams.set("sni", serverName);
  uri.searchParams.set("fp", "chrome");
  uri.searchParams.set("pbk", publicKey);
  uri.searchParams.set("sid", shortId);
  uri.searchParams.set("spx", "/");
  uri.searchParams.set("type", "tcp");
  uri.searchParams.set("headerType", "none");
  uri.hash = "Local Xray E2E";

  return {
    accessLog,
    configPath,
    privateKey,
    uri: uri.href,
  };
}

async function startWireGuardInfrastructure() {
  if (!dockerAvailable()) {
    throw new Error(
      "The network E2E suite requires a running Docker daemon for its local WireGuard peer",
    );
  }

  const port = await freePort();
  const name = `salamander-wg-e2e-${process.pid}-${Date.now()}`;
  const image =
    process.env.SALAMANDER_E2E_WIREGUARD_IMAGE ??
    "lscr.io/linuxserver/wireguard:latest";
  log("Starting isolated local WireGuard peer");
  runDocker([
    "run",
    "-d",
    "--name",
    name,
    "--cap-add=NET_ADMIN",
    "-p",
    `${port}:51820/udp`,
    "-e",
    "PEERS=1",
    "-e",
    "SERVERURL=127.0.0.1",
    "-e",
    "SERVERPORT=51820",
    "-e",
    "PEERDNS=auto",
    "-e",
    "INTERNAL_SUBNET=10.64.0.0",
    image,
  ]);

  try {
    const deadline = Date.now() + 45_000;
    let config = "";
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 1_000));
      const configResult = runDocker(
        ["exec", name, "cat", "/config/peer1/peer1.conf"],
        { allowFailure: true },
      );
      const statusResult = runDocker(["exec", name, "wg", "show"], {
        allowFailure: true,
      });
      if (
        configResult.status === 0 &&
        statusResult.status === 0 &&
        statusResult.stdout.includes("listening port")
      ) {
        config = configResult.stdout;
        break;
      }
    }
    if (!config) {
      throw new Error("Local WireGuard peer did not become ready within 45s");
    }

    const server = runDocker([
      "exec",
      "-d",
      name,
      "sh",
      "-c",
      'while true; do printf "HTTP/1.1 200 OK\\r\\nContent-Length: 13\\r\\nConnection: close\\r\\n\\r\\nWG-TUNNEL-OK\\n" | nc -l -p 8080 >> /tmp/salamander-e2e-target-requests 2>/dev/null; done',
    ]);
    if (server.status !== 0) {
      throw new Error("Failed to start the WireGuard tunnel target server");
    }
    config = config.replace(
      /^Endpoint\s*=.*$/m,
      `Endpoint = 127.0.0.1:${port}`,
    );
    return {
      name,
      config,
      targetUrl: "http://10.64.0.1:8080/salamander-e2e-wireguard",
    };
  } catch (error) {
    runDocker(["rm", "-f", name], { allowFailure: true });
    throw error;
  }
}

async function prepareRetainedArtifacts(
  runRoot,
  { suite, failed, sensitiveValues },
) {
  const sessionsRoot = path.join(runRoot, "sessions");
  const sessions = await readdir(sessionsRoot, {
    withFileTypes: true,
  }).catch(() => []);
  await Promise.all(
    sessions
      .filter((entry) => entry.isDirectory())
      .map((entry) =>
        rm(path.join(sessionsRoot, entry.name, "salamander", "data", "binaries"), {
          recursive: true,
          force: true,
        }),
      ),
  );
  await createSafeDiagnostics(runRoot, { suite, failed, sensitiveValues });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const runRoot = await mkdtemp(path.join(os.tmpdir(), "salamander-e2e-"));
  await mkdir(path.join(runRoot, "logs"), { recursive: true });
  const records = [];
  let fixture;
  let wireGuard;
  let xray;
  let failed = false;
  const sensitiveValues = [];
  const cleanup = async () => {
    await Promise.all(records.reverse().map(stopProcess));
    if (fixture) {
      await new Promise((resolve) => fixture.server.close(resolve));
    }
    if (wireGuard) {
      runDocker(["rm", "-f", wireGuard.name], { allowFailure: true });
    }
    if (xray) {
      await rm(xray.configPath, { force: true });
    }
    if (!options.keep && !failed) {
      await rm(runRoot, { recursive: true, force: true });
    } else {
      await prepareRetainedArtifacts(runRoot, {
        suite: options.suite,
        failed,
        sensitiveValues,
      });
      log(`Artifacts retained at ${runRoot}`);
    }
  };
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, () => {
      failed = true;
      cleanup().finally(() => process.exit(signal === "SIGINT" ? 130 : 143));
    });
  }

  try {
    log(`Run root: ${runRoot}`);
    if (options.build) {
      buildAll();
    } else if (!existsSync(appBinary) || !existsSync(driverBinary)) {
      throw new Error(
        "--no-build requested but the E2E app or driver binary is missing",
      );
    }

    const driverPort = await freePort();
    const driver = startProcess(
      "tauri-wd",
      driverBinary,
      [
        "--port",
        String(driverPort),
        "--max-sessions",
        "4",
        "--startup-timeout",
        "120",
        "--command-timeout",
        "630",
        "--log",
        options.verbose ? "debug" : "info",
      ],
      {
        cwd: projectRoot,
        env: process.env,
        runRoot,
        verbose: options.verbose,
      },
    );
    records.push(driver);
    await waitForUrl(`http://127.0.0.1:${driverPort}/status`, 15_000, driver);

    const needsBrowser =
      options.suite === "network" || options.suite === "full";
    const networkEnabled =
      (options.suite === "network" || options.suite === "full") &&
      process.env.SALAMANDER_E2E_SKIP_NETWORK_TEST !== "1";
    const geoIpFixture = needsBrowser ? await ensureGeoIpFixture() : null;
    fixture = await startFixtureServer(geoIpFixture);
    if (networkEnabled && process.env.SALAMANDER_E2E_SKIP_VPN_TUNNEL !== "1") {
      wireGuard = await startWireGuardInfrastructure();
    }
    if (networkEnabled) {
      xray = await startXrayInfrastructure(runRoot, options, records);
      sensitiveValues.push(xray.privateKey);
    }

    const localValues = await loadLocalValues([
      "WAYFERN_TEST_TOKEN",
      "RESIDENTIAL_PROXY_URL_ONE_SOCKS",
      "RESIDENTIAL_PROXY_URL_ONE_HTTP",
    ]);
    sensitiveValues.push(...Object.values(localValues));
    // WAYFERN_TEST_TOKEN authorized downloading the old "wayfern" browser
    // engine, which the app no longer supports (see fixtures.mjs's
    // prepareWayfern and the now-skipped tests in network.test.mjs). Nothing
    // that still runs needs this token, so it's no longer required.
    const token = localValues.WAYFERN_TEST_TOKEN ?? "";
    if (wireGuard) {
      sensitiveValues.push(
        wireGuard.config,
        Buffer.from(wireGuard.config).toString("base64"),
      );
    }

    const files = suiteFiles[options.suite]
      .filter((file) => networkEnabled || file !== "network.test.mjs")
      .map((file) => path.join(dirname, "tests", file));
    const testArgs = [
      "--test",
      "--test-concurrency=1",
      "--test-reporter=spec",
      ...files,
    ];
    const child = spawn(process.execPath, testArgs, {
      cwd: projectRoot,
      env: {
        ...process.env,
        SALAMANDER_E2E_RUN_ROOT: runRoot,
        SALAMANDER_E2E_PROJECT_ROOT: projectRoot,
        SALAMANDER_E2E_APP: appBinary,
        SALAMANDER_E2E_DRIVER_URL: `http://127.0.0.1:${driverPort}`,
        SALAMANDER_E2E_FIXTURE_URL: `http://127.0.0.1:${fixture.port}`,
        SALAMANDER_E2E_GEOIP_FIXTURE_READY: geoIpFixture ? "1" : "0",
        WAYFERN_TEST_TOKEN: token,
        RESIDENTIAL_PROXY_URL_ONE_SOCKS:
          localValues.RESIDENTIAL_PROXY_URL_ONE_SOCKS ?? "",
        RESIDENTIAL_PROXY_URL_ONE_HTTP:
          localValues.RESIDENTIAL_PROXY_URL_ONE_HTTP ?? "",
        SALAMANDER_E2E_WIREGUARD_CONFIG_BASE64: wireGuard
          ? Buffer.from(wireGuard.config).toString("base64")
          : "",
        SALAMANDER_E2E_WIREGUARD_TARGET_URL: wireGuard?.targetUrl ?? "",
        SALAMANDER_E2E_WIREGUARD_CONTAINER: wireGuard?.name ?? "",
        SALAMANDER_E2E_VLESS_URI: xray?.uri ?? "",
        SALAMANDER_E2E_XRAY_ACCESS_LOG: xray?.accessLog ?? "",
      },
      stdio: "inherit",
    });
    const exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => {
        resolve(code ?? (signal ? 1 : 0));
      });
    });
    if (exitCode !== 0) {
      throw new Error(
        `E2E suite ${options.suite} failed with status ${exitCode}`,
      );
    }
    log(`Suite ${options.suite} passed`);
  } catch (error) {
    failed = true;
    process.stderr.write(`[salamander-e2e] ERROR: ${error.stack ?? error}\n`);
    process.exitCode = 1;
  } finally {
    await cleanup();
  }
}

await main();
