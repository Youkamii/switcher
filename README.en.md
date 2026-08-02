<h1><img src="docs/logo.svg" width="26" alt="" /> switcher</h1>

[한국어](README.md) | **English** | [日本語](README.ja.md) | [简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md) | [हिन्दी](README.hi.md)

A desktop widget that switches between multiple Claude Code / Codex CLI accounts in one click, with per-account usage bars (Windows·macOS).

<p align="center"><img src="docs/screenshot.png" alt="switcher — Type 1 / 2 / 3" /></p>
<p align="center"><sub>Three view modes — Type 1 (full) · Type 2 (widget) · Type 3 (compact)</sub></p>

## Windows

### Install

**Install via npm (recommended — no security warnings)** — requires Node.js 18+

```sh
npm install -g https://github.com/Youkamii/switcher/releases/latest/download/switcher-npm.tgz
switcher
```

The first run of the `switcher` command automatically downloads the latest release build (subsequent runs start instantly). Since it isn't a browser download, no SmartScreen warning appears. To update, quit the widget and run the same install command again.

**Direct download** — grab `switcher-win-x64.zip` from the [releases page](https://github.com/Youkamii/switcher/releases/latest), extract it, and run `switcher.exe`. (Windows 10/11, 64-bit)

- The binary is not code-signed, so Windows SmartScreen may show an "unknown publisher" warning on first launch. Click `More info` → `Run anyway`.
- The webview uses WebView2, which ships with Windows.

### Run

- While running, it lives in the tray (right side of the taskbar) as a W icon. Closing the window (Alt+F4) does not quit it.
- Left-click the W tray icon to bring the window back. To quit completely, right-click the tray icon → Quit.
- Change the UI language via right-click on the tray icon → Settings → Language (한국어·English·日本語·简体中文·繁體中文·हिन्दी).
- To start it at boot, press `Win+R` → `shell:startup` and drop a shortcut to the exe into that folder.

## macOS

### Install

**Install via npm (recommended — no security warnings)** — requires Node.js 18+

```sh
npm install -g https://github.com/Youkamii/switcher/releases/latest/download/switcher-npm.tgz
switcher
```

The first run of the `switcher` command automatically downloads the latest release build (subsequent runs start instantly). Since it isn't a browser download, no "unidentified developer" warning appears. To update, quit the widget and run the same install command again.

**Direct download** — grab `switcher-mac-arm64.zip` from the [releases page](https://github.com/Youkamii/switcher/releases/latest), extract it, and run `switcher.app`. Apple Silicon only — on Intel Macs, install via [Build from source](#build-from-source) below.

- The app is not code-signed, so the first launch may be blocked with an "unidentified developer" message. Open System Settings → Privacy & Security, scroll to the bottom, and click **Open Anyway**.

### Run

- Launch `switcher.app`. It does not appear in the Dock or Cmd+Tab; it lives in the right side of the menu bar as a W icon.
- The widget stays on top across all desktops (Spaces) and even over full-screen apps.
- Left-click the menu bar W icon to toggle the window; right-click → Quit to exit completely.
- Changing the UI language on macOS is **under development** — the UI is currently shown in Korean.
- To start it at boot, add `switcher.app` under System Settings → General → Login Items.

## Using the widget (Windows·macOS)

<table align="center">
<tr>
<td align="center" width="450">
<img src="docs/demo.gif" width="420" alt="Widget mode demo — double-click an account card to switch; clicks on empty areas pass through to the window behind" />
</td>
<td width="430">

**Widget mode behavior**

- **Double-click** an account card → switches auth to that account
- Clicks and drags outside the cards **pass through to the window behind**
- The active account is shown in higher saturation
- Move the window with the ☰ handle; cycle modes with the Type button at the top right
- On macOS, it stays visible across all Spaces and over full-screen apps

</td>
</tr>
</table>

## Overview

Whether you use Claude Code or Codex, a terminal only holds one login at a time. Multi-account users re-run `/login` every time a limit fills up, go through browser auth again, and lose track of which account is active.

switcher removes that loop. Log in once per account, and from then on switching is a single click in the widget. Each account's usage (5-hour and weekly limits) is shown as bars, so you can see which account has headroom and hop over.

## Features

- Account switching: one click, no re-login. Applies to newly opened terminals.
- Usage display: per account, 5 Hours / Weekly / per-model limits with time remaining until reset.
- Add accounts: open the login link shown in the widget, get a code, paste it in.
- Subscription tier: Max (5x yellow, 20x red) / Pro / Plus badges next to each account.
- Modes (Type1/2/3): full → widget → compact cycle. In widget/compact modes the buttons hide, clicks and drags pass through to the window behind, and double-clicking a card switches accounts. Move the window with the ☰ handle.
- The window height auto-fits the content. Lowering the opacity slider fades the background first, then the frame.
- UI language: tray → Settings → Language, 6 languages (Korean·English·Japanese·Simplified Chinese·Traditional Chinese·Hindi). Under development on macOS.

## How it works

Both CLIs store their login token locally.

- Claude Code: `~/.claude/.credentials.json` (Windows) / on macOS, the **Keychain** item "Claude Code-credentials"
- Codex CLI: `~/.codex/auth.json` (same on both OSes)

On macOS, switcher reads and writes the Keychain the same way the Claude CLI does (via the built-in `security` tool) — no extra permission popups.

switcher keeps per-account tokens as profiles under `~/.switcher/` and swaps files in two steps when switching:

1. Back up the currently active file into the current account's profile. Tokens refresh themselves frequently, so this step must come first.
2. Copy the target account's profile into the active location.

Note: if a CLI session is running in a terminal, it's safest to finish it before switching. A live session that auto-refreshes its token may rewrite the active file, overwriting the account you just switched to with the previous account's token.

Chat history, memory, and settings live in local folders unrelated to the account, so your work environment stays intact across switches.

Usage is queried directly from the same usage API the CLI uses, with each account's token. A 60-second cache avoids rate limits. If a query is blocked, the last known values are shown.

Claude access tokens only live a few hours, so when a stored profile's token expires the widget re-issues it the same way the CLI does and writes it back to the profile — all profiles once at app start, then on demand per query. That keeps usage live even for accounts you aren't using. The token of the account currently in use is refreshed by the CLI itself, so the widget leaves it alone.

Adding an account is handled with an isolated login.

## Adding an account

Press "＋ Add account" in the widget and a login URL appears. Paste that URL into any browser you like.

- **Claude**: after logging in, the browser shows a code. Paste that code into the widget's input field and you're done.
- **Codex**: the widget shows the URL together with a one-time code (valid for 15 minutes). Enter that code in the browser and the rest is automatic.

**Before adding Codex for the first time**: device-code authentication is disabled by default on OpenAI accounts. If it's off, entering the code gets rejected with "enable device code authentication and try again".

- Personal accounts: chatgpt.com → profile → Settings → Security (or Data Controls) → enable **Codex device code authentication**
- Team/Business accounts: an admin enables it under Workspace Settings → Permissions & Roles

Note: the Claude CLI tries to open your default browser once when the login starts. You can close that window and continue in the browser where you pasted the widget's URL.

## Tech

Tauri 2 + Rust, with a vanilla TypeScript frontend. Account switching, usage queries, and isolated logins are all handled in Rust.
Tokens never reach the webview.
CLI login screens are read through a virtual console (PTY).

## Build from source

To build from source instead of downloading, you need the [Node.js](https://nodejs.org) and [Rust](https://rustup.rs) toolchains.

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm run setup
```

`npm run setup` installs dependencies and builds the app in one go. Instead of dumping verbose logs, it shows a spinner and elapsed time.

The first build compiles all of Rust, so it **can take 5–10 minutes.** It isn't stuck — just wait. The output lands at `src-tauri\target\release\switcher.exe` on Windows and `src-tauri/target/release/bundle/macos/switcher.app` on macOS — feel free to move the app into your Applications folder.

For development, run `npm run tauri dev`.

---

<div align="center">
<sub>Licensed under the <a href="LICENSE">MIT License</a> — free for any use, including commercial. Keep the copyright and license notice.</sub>
</div>
