# Building Salamander Browser on macOS

Covers installing every dependency from a clean machine, compiling a local
build, and installing the result. Written for the `add_auth` branch but
applies to any branch of this repo.

## 1. Xcode Command Line Tools

Provides the C/Objective-C toolchain Tauri and several Rust crates
(`objc2`, `objc2-app-kit`, `core-foundation`) need to compile.

```bash
xcode-select --install
```

A system dialog appears — click Install and wait (10–15 minutes on a clean
machine). Verify:

```bash
xcode-select -p
# -> /Library/Developer/CommandLineTools (or /Applications/Xcode.app/...)
```

## 2. Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Choose option **1) Proceed with standard installation**. Then either
restart the terminal or run:

```bash
source "$HOME/.cargo/env"
```

Verify:

```bash
rustc --version
cargo --version
```

## 3. Node.js and pnpm

This project targets Node **24** (see `.node-version`).

Install Node 24 via [nodejs.org](https://nodejs.org/) or `nvm`:

```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
# restart the terminal, then:
nvm install 24
nvm use 24
```

Enable pnpm via Corepack — this resolves the exact pnpm version pinned in
`package.json` (`packageManager`) automatically, no manual version pinning
needed:

```bash
corepack enable
```

Verify:

```bash
node --version   # v24.x
pnpm --version   # 11.21.0
```

## 4. Get the code

```bash
git clone git@github.com:mb-technical-solutions/salamanderbrowser.git
cd salamanderbrowser
git checkout add_auth
```

(Skip this if the code was already handed to you as a folder — just `cd`
into it.)

## 5. Install project dependencies

```bash
pnpm install
```

JS dependencies only — Rust dependencies are fetched and compiled by the
next step.

## 6. Compile

```bash
pnpm tauri build
```

This single command builds the frontend, compiles and copies the
`salamander-proxy` sidecar and downloads the verified Xray-core sidecar for
your Mac's own architecture (detected automatically — Apple Silicon or
Intel), then compiles and packages the app itself. The first run compiles
every Rust dependency from scratch and takes a while; later runs are much
faster thanks to incremental caching.

Output:

```
src-tauri/target/release/bundle/dmg/Salamander Browser_1.0.0_<arch>.dmg
src-tauri/target/release/bundle/macos/Salamander Browser.app
```

(`<arch>` is `aarch64` on Apple Silicon, `x86_64` on Intel.)

## 7. Install the built app

1. Open the `.dmg` from `bundle/dmg/`.
2. Drag **Salamander Browser.app** into the **Applications** folder shown in
   the window.
3. Launch it from Launchpad or Finder → Applications.

### First launch

This build is unsigned (no Apple Developer certificate), so a normal
double-click shows "the app is damaged" or "cannot verify the developer."
Instead:

- Right-click the app → **Open** → confirm **Open** again in the dialog.
- If that doesn't appear, open System Settings → Privacy & Security, scroll
  down, and click **Open Anyway** after the first failed launch attempt.

When signing in through Authentik for the first time, macOS will ask for
permission to use the **Keychain** — this is where the refresh token is
stored so later launches don't require a password. Allow it, or sign-in
state won't persist between launches.

## Why this can't be cross-compiled from Windows or Linux

`cargo check --target x86_64-apple-darwin` (or `aarch64-apple-darwin`) fails
on any non-Mac host, including with the correct Rust target installed via
`rustup target add`, because `objc2-exception-helper` (pulled in
transitively by `objc2-app-kit`, already a dependency before this feature)
compiles real Objective-C source in its `build.rs` and needs Apple's own
`cc`/Clang and macOS SDK — there's no cross-toolchain substitute for it set
up in this repo. This is a property of the existing codebase, not something
introduced by the auth feature. A real Mac (or the project's own macOS CI
runners) is the only way to produce a working macOS build.
