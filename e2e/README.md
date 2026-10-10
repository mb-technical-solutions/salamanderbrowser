# Salamander Browser native E2E tests

These tests exercise the actual Tauri application through the published
[`tauri-wd`](https://crates.io/crates/tauri-wd) native test driver. They do
not replace Rust or React unit tests; they
cover the process boundaries those tests cannot: WKWebView/WebView2/WebKitGTK UI, Tauri invokes,
and child-process cleanup.

> **Note:** `e2e:network`'s two browser-launch tests (VLESS/Xray-core routing and the
> visible profile/proxy/VPN/extension UI flow) are currently `test.skip`-ed. They were written
> against "Wayfern", a Chromium-based engine this app no longer ships (it now only supports
> Camoufox — see `get_supported_browsers()`), and downloading it via `prepareWayfern` in
> `e2e/lib/fixtures.mjs` no longer works. Rewriting them against Camoufox is a separate task.
> The `browser` suite that downloaded and drove Wayfern directly has been removed entirely.

## Local setup

Install Salamander's dependencies with `pnpm install`. The runner installs the driver itself with
`cargo install`, so a working Rust toolchain is the only extra requirement.

The real-network suite additionally requires Docker plus
`RESIDENTIAL_PROXY_URL_ONE_HTTP` and `RESIDENTIAL_PROXY_URL_ONE_SOCKS`. It creates its own
WireGuard server and tunnel-only HTTP target in a disposable container. It never connects a test
profile to a developer or production VPN.

Run one suite:

```sh
pnpm e2e:smoke
pnpm e2e:ui
pnpm e2e:entities
pnpm e2e:network
pnpm e2e:integrations
```

Run everything with `pnpm e2e`. A normal run builds the Next frontend, `salamander-proxy`, and the
harness in `e2e/app`, then installs the `tauri-wd` CLI into the ignored `e2e/.driver` root when the
version pinned by `e2e/app/Cargo.lock` is not already there. The harness enables Salamander's `e2e`
feature and injects the WebDriver plugin so the production crate never depends on it. Both the
plugin and the CLI come from the same pinned crates.io release, so they cannot drift apart. Bump
the pin in `e2e/app/Cargo.toml` to move to a newer driver. Add `--no-build` to
`node e2e/run.mjs --suite=<name>` only when all four outputs are current.
`SALAMANDER_E2E_KEEP_ARTIFACTS=1` retains successful local runs; failed runs are always retained and
their location is printed. Raw screenshots, captured HTML, logs, and isolated app state stay local.
The runner also creates a text-only `diagnostics/` directory whose logs are redacted and checked
against active test secrets. CI uploads only that directory on failure. Disposable copied browser
binaries are pruned so repeated failures do not consume gigabytes.

The suites deliberately distinguish visible behavior from command coverage. `e2e:entities`
exercises isolated CRUD and persistence through Tauri commands. `e2e:network` is meant to visibly
create a profile group, HTTP proxy, WireGuard VPN, extension, and extension group; assign the proxy
and VPN in the profile table; validate both residential HTTP and SOCKS5 proxies; then launch a
browser through the residential proxy and through the local WireGuard tunnel — see the note above
for why that last part is currently skipped. `e2e:ui` selects predefined, preset, and manually
customized themes through the native UI and asserts their persisted settings and rendered CSS
variables across rail navigation and app restart.

## Isolation contract

Each app session receives a unique root under the operating-system test temp directory. The
runner redirects:

- Salamander data, cache, and logs with `SALAMANDERBROWSER_DATA_ROOT`;
- `HOME`, `USERPROFILE`, `CFFIXED_USER_HOME`, XDG paths, `APPDATA`, and `LOCALAPPDATA`;
- `TMPDIR`, `TMP`, and `TEMP`;
- the Tauri WebView store (incognito for WKWebView, whose persistent data-directory API is not
  honored);
- all WebDriver and fixture ports.

The E2E feature suppresses automatic updater/download traffic. Production builds never include
the WebDriver plugin.

## CI

`.github/workflows/app-e2e.yml` runs smoke tests on macOS, Linux/Xvfb, and Windows for pull
requests. Pushes to `main`, weekly schedules, and manual runs execute the full macOS suite, plus a
Linux/Docker job for residential proxy and local WireGuard browser traffic.

Every job restores the compiled driver from an `actions/cache` entry keyed by `e2e/app/Cargo.lock`,
the same file the runner reads the version from, so only a driver bump pays for a rebuild. The
network job requires `RESIDENTIAL_PROXY_URL_ONE_HTTP` and `RESIDENTIAL_PROXY_URL_ONE_SOCKS`.
