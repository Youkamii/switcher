<h1><img src="logo.svg" width="26" alt="" /> switcher</h1>

[한국어](../README.md) | [English](README.en.md) | [日本語](README.ja.md) | **简体中文** | [繁體中文](README.zh-TW.md) | [हिन्दी](README.hi.md)

一键切换 Claude Code 与 Codex CLI 账号的桌面小组件（Windows·macOS）。

<p align="center"><img src="screenshot.png" alt="switcher — Type 1 / 2 / 3" /></p>
<p align="center"><sub>三种视图模式 — Type 1（完整）· Type 2（小组件）· Type 3（紧凑）</sub></p>

## Windows

### 安装

**通过 npm 安装（推荐 — 无安全警告）** — 需要 Node.js 18 及以上

```sh
npm install -g switcher-widget
switcher
```

`switcher` 命令首次运行时会自动下载最新的发布版构建（之后会直接启动）。由于不是通过浏览器下载，不会触发 SmartScreen 警告。更新是自动的 — 每次启动都会检查新版本，下次启动生效。

**直接下载** — 从[发布页](https://github.com/Youkamii/switcher/releases/latest)下载 `switcher-win-x64.zip`，解压后运行 `switcher.exe`。（Windows 10/11 64 位）

- 由于没有代码签名，首次运行时 Windows SmartScreen 可能会弹出“未知发布者”警告。点击 `更多信息` → `仍要运行`。
- 网页视图使用 Windows 自带的 WebView2。

### 运行

- 运行期间会以 W 图标常驻托盘（任务栏右侧）。即使关闭窗口（Alt+F4）也不会退出。
- 左键点击托盘的 W 图标可重新唤出窗口。要完全退出，右键点击托盘图标 → 退出。
- UI 语言可在右键点击托盘图标 → 设置 → 语言中切换（한국어·English·日本語·简体中文·繁體中文·हिन्दी）。
- 首次运行时会自动在桌面创建 `switcher` 快捷方式（删除后不会重新创建）。
- 开机自启动默认开启 — 可在托盘 设置 → 开机自启动 中关闭。
- 每次启动都会检查新版本并自动更新（下次启动生效）— 可在托盘 设置 → 自动更新 中关闭。

## macOS

### 安装

**通过 npm 安装（推荐 — 无安全警告）** — 需要 Node.js 18 及以上

```sh
npm install -g switcher-widget
switcher
```

`switcher` 命令首次运行时会自动下载最新的发布版构建（之后会直接启动）。由于不是通过浏览器下载，不会触发“无法验证开发者”警告。更新时先退出小组件，再重新执行同一条安装命令即可。

**直接下载** — 从[发布页](https://github.com/Youkamii/switcher/releases/latest)下载 `switcher-mac-arm64.zip`，解压后运行 `switcher.app`。仅支持 Apple Silicon — Intel Mac 请通过下方的[从源码构建](#从源码构建)安装。

- 由于没有代码签名，首次打开时可能会提示“无法验证开发者”而被拦截。前往 系统设置 → 隐私与安全性，在页面最底部点击**仍要打开**即可运行。

### 运行

- 运行 `switcher.app`。它不会出现在 Dock 和 Cmd+Tab 中，而是以 W 图标常驻菜单栏右侧。
- 小组件会悬浮显示在所有桌面（Space）和全屏应用之上。
- 左键点击菜单栏的 W 图标可切换窗口的打开/隐藏，要完全退出，右键点击 → 退出。
- UI 语言切换功能**开发中** — 目前以韩语显示。
- 要开机自启，前往 系统设置 → 通用 → 登录项，添加 `switcher.app`。

## 小组件使用方法（Windows·macOS 通用）

<table align="center">
<tr>
<td align="center" width="450">
<img src="demo.gif" width="420" alt="小组件模式演示 — 双击账号卡片切换，空白区域点击穿透到后方窗口" />
</td>
<td width="430">

**小组件模式的行为**

- **双击**账号卡片 → 切换到该账号的认证
- 卡片以外的点击·拖拽会**直接穿透到后方窗口**
- 当前激活的账号以高饱和度显示
- 移动窗口用 ☰ 手柄，循环切换模式用右上角的 Type 按钮
- 在 Mac 上还会悬浮显示在所有桌面（Space）和全屏应用之上

</td>
</tr>
</table>

## 概述

无论 Claude Code 还是 Codex，在同一个终端里都只能登录一个账号。多账号用户每当额度用满，就得重新 `/login`、重新走一遍浏览器认证，还常常搞不清当前用的是哪个账号。

switcher 省掉了这个过程。每个账号只需首次登录一次，之后在小组件里点一下按钮即可切换。各账号的用量（5 小时·每周额度）以进度条显示，看哪个账号还有余量，切换过去就行。

## 功能

- 账号切换：无需重新登录，一键完成。从新打开的终端开始生效。
- 用量显示：每个账号都能看到 5 Hours / Weekly / 各模型的额度，以及距离重置的剩余时间。
- 添加账号：通过小组件中显示的登录链接获取代码后输入即可。
- 订阅级别：账号旁会标注 Max（5x 为黄色，20x 为红色）/ Pro / Plus。
- 模式（Type1/2/3）：按完整 → 小组件 → 紧凑循环切换。在小组件·紧凑模式下按钮会隐藏，点击·拖拽穿透到后方窗口，双击账号卡片即可切换账号。移动窗口用 ☰ 手柄。
- 窗口高度会根据内容自动调节。调低透明度滑块时，背景先变淡，框架随后变淡。
- UI 语言：在托盘 → 设置 → 语言中可切换 6 种语言（韩语·英语·日语·简体中文·繁体中文·印地语）。macOS 版开发中。
- 自动更新·开机自启动·桌面快捷方式（Windows）：在托盘设置中开关。macOS 版开发中。
- GitHub 账号切换：在小组件中切换已登录 gh CLI 的账号 — git push/pull（HTTPS）跟随活动账号。无用量显示。
- 黑屏模式（Windows）：🌙 按钮或托盘菜单把所有屏幕盖上置顶黑幕。移动鼠标时光标周围像雾散开一样短暂透出画面；快速晃动鼠标或按 ESC 退出。macOS 版开发中。

## 工作原理

两个 CLI 都把登录令牌保存在本地。

- Claude Code：`~/.claude/.credentials.json`（Windows）/ macOS 为**钥匙串**中的 “Claude Code-credentials” 条目
- Codex CLI：`~/.codex/auth.json`（两个系统相同）

在 Mac 上，switcher 以与 Claude CLI 相同的方式（macOS 内置的 `security` 工具）读写钥匙串 — 无需额外的权限弹窗即可工作。

switcher 把各账号的令牌以配置文件形式保存在 `~/.switcher/` 下，切换时分两步替换文件。

1. 先把当前活动文件备份到当前账号的配置文件中。由于令牌会随时自动刷新，这一步必须在前。
2. 再把目标账号的配置文件复制到活动位置。

注意：如果终端里还有 CLI 会话在运行，建议先结束再切换。留着的会话在自动刷新令牌时会重写活动文件，刚切换好的账号可能被旧账号的令牌覆盖。

对话记录·记忆·设置都存放在与账号无关的本地文件夹里，切换账号后工作环境保持不变。

用量通过各账号的令牌直接查询 CLI 所用的用量 API。为避免触发请求限制，设有 60 秒缓存。查询失败时显示上一次的值。

Claude 的访问令牌寿命只有几个小时，因此当保管的配置文件中的令牌过期时，小组件会以与 CLI 相同的方式重新签发并写回配置文件 — 启动应用时全部刷新一次，之后仅在查询时按需刷新。所以未使用账号的用量也始终保持实时。当前使用中账号的令牌由 CLI 自行刷新，小组件不会去动它。

添加账号通过隔离登录处理。

## 添加账号

点击小组件中的“＋ 添加账号”会显示登录地址。把该地址粘贴到任意浏览器中。

- **Claude**：在浏览器中登录后，页面上会显示一个代码。把它粘贴到小组件的输入框即可。
- **Codex**：小组件会显示地址和一次性代码（15 分钟有效）。在浏览器中输入该代码，剩下的会自动完成。

**首次添加 Codex 之前**：设备代码认证在 OpenAI 账号中默认是关闭的。若不开启，即使输入代码也会被拒绝，提示“请先启用设备代码认证后重试”。

- 个人账号：chatgpt.com → 个人资料 → 设置 → 安全（或数据控制）→ 开启 **Codex 设备代码认证**
- 团队·企业账号：由管理员在工作区设置 → 权限与角色中启用

备注：Claude CLI 在开始登录时会尝试打开一次默认浏览器。那个窗口可以直接关掉，在粘贴了小组件地址的浏览器中继续即可。

## GitHub 账号切换

安装了 [GitHub CLI (gh)](https://cli.github.com) 后，小组件中会出现 GITHUB 区域。每个账号在终端执行一次 `gh auth login`，之后就能在小组件中切换 — 内部走与 `gh auth switch` 相同的通道，并在每次切换时执行 `gh auth setup-git`，让 git push/pull（HTTPS）跟随活动账号。令牌由 gh 保存在 keyring 中，小组件不会接触。

已知限制：

- SSH 远程（`git@github.com:...`）由 SSH 密钥决定身份，不受此切换影响。仅 HTTPS 远程有效。
- 提交作者（`git config user.name/email`）不会改变 — 切换后提交仍保留原有名字。
- VS Code、Copilot 等其他应用的 GitHub 会话使用各自的令牌，不会跟随。
- 使用 SAML SSO 的组织仓库需要按账号完成 SSO 授权才能访问。
- 每次切换执行的 `gh auth setup-git` 会在全局 git 配置中为 github.com 永久注册 gh 作为 credential helper，替换现有的 GCM 设置 — 撤销：`git config --global --unset-all credential.https://github.com.helper`。

## 技术

Tauri 2 + Rust，前端为原生 TypeScript（vanilla）。账号切换·用量查询·隔离登录全部在 Rust 中处理。
令牌不会进入网页视图。
CLI 的登录界面通过虚拟控制台（PTY）读取。

## 从源码构建

如果不想直接下载而是从源码构建，需要 [Node.js](https://nodejs.org) 和 [Rust](https://rustup.rs) 工具链。

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm run setup
```

`npm run setup` 一次完成依赖安装和应用构建。它不会刷出冗长的日志，只显示加载指示和已用时间。

首次构建需要完整编译 Rust，**可能耗时 5~10 分钟。** 加载并没有卡住，耐心等待即可。产物为 Windows `src-tauri\target\release\switcher.exe`、macOS `src-tauri/target/release/bundle/macos/switcher.app` — 也可以把应用移动到“应用程序”文件夹。

开发运行使用 `npm run tauri dev`。

---

<div align="center">
<sub>Licensed under the <a href="../LICENSE">MIT License</a> — free for any use, including commercial. Keep the copyright and license notice.</sub>
</div>
