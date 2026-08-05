<h1><img src="logo.svg" width="26" alt="" /> switcher</h1>

[한국어](../README.md) | [English](README.en.md) | [日本語](README.ja.md) | [简体中文](README.zh-CN.md) | **繁體中文** | [हिन्दी](README.hi.md)

一鍵切換 Claude Code 與 Codex CLI 帳號的桌面小工具(Windows·macOS)。

<p align="center"><img src="screenshot.png" alt="switcher — Type 1 / 2 / 3" /></p>
<p align="center"><sub>三種檢視模式 — Type 1(完整)· Type 2(小工具)· Type 3(精簡)</sub></p>

## Windows

### 安裝

**以 npm 安裝(建議 — 無安全性警告)** — 需 Node.js 18 以上

```sh
npm install -g switcher-widget
switcher
```

第一次執行 `switcher` 指令時,會自動下載最新發行版本的建置檔(之後就會直接啟動)。因為不是透過瀏覽器下載,所以不會出現 SmartScreen 警告。更新是自動的 — 每次啟動都會檢查新版本,下次啟動生效。

**直接下載** — 從[發行版本](https://github.com/Youkamii/switcher/releases/latest)下載 `switcher-win-x64.zip`,解壓縮後執行 `switcher.exe`。(Windows 10/11 64 位元)

- 因為沒有程式碼簽章,第一次執行時 Windows SmartScreen 可能會顯示「未知的發行者」警告。點選 `其他資訊` → `仍要執行`。
- WebView 使用 Windows 內建的 WebView2。

### 執行

- 執行期間會以 W 圖示常駐在系統匣(工作列右側)。即使關閉視窗(Alt+F4)也不會結束。
- 要重新叫出視窗,左鍵點擊系統匣的 W 圖示;要完全結束,右鍵點擊系統匣圖示 → 結束。
- UI 語言可在右鍵點擊系統匣圖示 → 設定 → 語言中切換(한국어·English·日本語·简体中文·繁體中文·हिन्दी)。
- 首次執行時會自動在桌面建立 `switcher` 捷徑(刪除後不會重新建立)。
- 開機自動啟動預設開啟 — 可在系統匣 設定 → 開機自動啟動 中關閉。
- 每次啟動都會檢查新版本並自動更新(下次啟動生效) — 可在系統匣 設定 → 自動更新 中關閉。

## macOS

### 安裝

**以 npm 安裝(建議 — 無安全性警告)** — 需 Node.js 18 以上

```sh
npm install -g switcher-widget
switcher
```

第一次執行 `switcher` 指令時,會自動下載最新發行版本的建置檔(之後就會直接啟動)。因為不是透過瀏覽器下載,所以不會出現「未識別的開發者」警告。要更新時,先結束小工具,再重新執行同一個安裝指令即可。

**直接下載** — 從[發行版本](https://github.com/Youkamii/switcher/releases/latest)下載 `switcher-mac-arm64.zip`,解壓縮後執行 `switcher.app`。僅支援 Apple Silicon — Intel Mac 請改用下方的[從原始碼建置](#從原始碼建置)安裝。

- 因為沒有程式碼簽章,第一次打開時可能會以「未識別的開發者」為由被擋下。請到系統設定 → 隱私權與安全性,點擊最下方出現的**強制打開**來執行。

### 執行

- 執行 `switcher.app`。它不會出現在 Dock 與 Cmd+Tab,而是以 W 圖示常駐在選單列右側。
- 小工具會覆疊顯示在所有桌面(Space)與全螢幕 App 之上。
- 左鍵點擊選單列的 W 圖示可切換視窗的開啟/隱藏;要完全結束,右鍵點擊 → 結束。
- UI 語言切換**開發中** — 目前僅以韓文顯示。
- 若要開機自動啟動,請到系統設定 → 一般 → 登入項目加入 `switcher.app`。

## 小工具使用方式(Windows·macOS 通用)

<table align="center">
<tr>
<td align="center" width="450">
<img src="demo.gif" width="420" alt="小工具模式示範 — 雙擊帳號卡片即切換,空白區域的點擊會穿透到後方視窗" />
</td>
<td width="430">

**小工具模式的運作**

- **雙擊**帳號卡片 → 切換為該帳號的認證
- 卡片以外的點擊與拖曳會**直接穿透到後方視窗**
- 目前啟用的帳號會以較高的飽和度顯示
- 移動視窗用 ☰ 把手,切換模式用右上角的 Type 按鈕
- 在 Mac 上也會覆疊顯示在所有桌面(Space)與全螢幕 App 之上

</td>
</tr>
</table>

## 概觀

不論是 Claude Code 還是 Codex,在同一部終端機裡一次只能登入一個帳號。擁有多個帳號的使用者每次額度用滿,就得重新 `/login`、重新走一次瀏覽器認證,還常常搞不清楚現在用的是哪個帳號。

switcher 把這整段流程省掉。每個帳號只要最初登入一次,之後在小工具裡按一個按鈕就能切換。各帳號的用量(5 小時·每週額度)以長條顯示,看哪個帳號還有餘裕,換過去就行。

## 功能

- 帳號切換:免重新登入,一鍵完成。從新開啟的終端機開始生效。
- 用量顯示:每個帳號都能看到 5 Hours / Weekly / 各模型的額度,以及距離重置的剩餘時間。
- 新增帳號:透過小工具顯示的登入連結取得代碼後輸入即可。
- 訂閱等級:帳號旁會標示 Max(5x 為黃色、20x 為紅色)/ Pro / Plus。
- 模式(Type1/2/3):依完整 → 小工具 → 精簡循環切換。在小工具與精簡模式下按鈕會隱藏,點擊與拖曳會穿透到後方視窗,雙擊帳號卡片即可切換帳號。移動視窗用 ☰ 把手。
- 視窗高度會依內容自動調整。調低透明度滑桿時,背景會先變淡,框架其後才變淡。
- UI 語言:在系統匣 → 設定 → 語言中可切換 6 種語言(韓文、英文、日文、簡體中文、繁體中文、印地文)。macOS 版開發中。
- 自動更新、開機自動啟動、桌面捷徑(Windows):在系統匣設定中開關。macOS 版開發中。
- GitHub 帳號切換:在小工具中切換已登入 gh CLI 的帳號 — git push/pull(HTTPS)會跟隨作用中帳號。無用量顯示。
- 黑屏模式:🌙 按鈕或系統匣選單把所有螢幕蓋上置頂黑幕。移動滑鼠時游標周圍像煙霧散開般透出畫面;持續用力晃動滑鼠 1~2 秒或按 ESC 解除——光會從最後的游標位置擴散開、揭開黑幕。macOS 上無法覆蓋全螢幕應用程式。
- 隱藏帳號資訊:🙈 按鈕將卡片上的電子郵件與 GitHub 帳號名模糊處理 — 防止螢幕分享·截圖外洩。再按一次恢復。
- 螢幕亮度調整(Windows):DISPLAY 區域的每台螢幕滑桿透過 DDC/CI 調整真實背光。若螢幕未開啟 DDC/CI 會顯示提示。macOS 版開發中。

## 運作方式

兩個 CLI 都把登入權杖儲存在本機。

- Claude Code:`~/.claude/.credentials.json`(Windows)/ macOS 則是**鑰匙圈**中的「Claude Code-credentials」項目
- Codex CLI:`~/.codex/auth.json`(兩個作業系統相同)

在 Mac 上,switcher 以與 Claude CLI 相同的方式(macOS 內建的 `security` 工具)讀寫鑰匙圈 — 不會跳出額外的權限視窗。

switcher 把各帳號的權杖以設定檔形式保存在 `~/.switcher/` 之下,切換時分兩個步驟替換檔案。

1. 先把目前的作用中檔案備份到現在這個帳號的設定檔。權杖會隨時自動更新,所以這一步必須在前。
2. 再把目標帳號的設定檔複製到作用中位置。

注意:如果終端機裡還有 CLI 工作階段在執行,先結束再切換比較安全。留著的工作階段在自動更新權杖時會重寫作用中檔案,剛切換好的帳號可能因此被前一個帳號的權杖蓋掉。

對話紀錄、記憶與設定都放在與帳號無關的本機資料夾,所以就算切換帳號,工作環境也維持原樣。

用量是以各帳號的權杖直接查詢 CLI 所使用的用量 API。為了避免觸發請求限制,設有 60 秒快取。查詢被擋下時,會顯示前一次的數值。

Claude 的存取權杖壽命只有幾個小時,所以當保管庫設定檔裡的權杖過期時,小工具會以與 CLI 相同的方式重新取得並寫回設定檔 — 啟動 App 時整批更新一次,之後只在查詢時更新需要的部分。因此就連沒在用的帳號,用量也始終是即時的。目前使用中帳號的權杖由 CLI 自行更新,小工具不會去動它。

新增帳號則以隔離登入處理。

## 新增帳號

按下小工具的「＋ 新增帳號」就會出現登入網址。把該網址貼到你想用的瀏覽器。

- **Claude**:在瀏覽器完成登入後,畫面上會出現一組代碼。把代碼貼到小工具的輸入欄就完成了。
- **Codex**:小工具會連同網址一起顯示一組一次性代碼(15 分鐘內有效)。在瀏覽器輸入該代碼後,其餘步驟會自動完成。

**第一次新增 Codex 之前**:裝置代碼認證在 OpenAI 帳號中預設是關閉的。若未開啟,即使輸入代碼也會被以「請先啟用裝置代碼認證後再重試」為由拒絕。

- 個人帳號:chatgpt.com → 個人檔案 → 設定 → 安全性(或資料控制)→ 開啟 **Codex 裝置代碼認證**
- 團隊·企業帳號:由管理員在工作區設定 → 權限與角色中啟用

附註:Claude CLI 在開始登入時會嘗試打開一次預設瀏覽器。那個視窗關掉也沒關係,直接在貼上小工具網址的瀏覽器裡進行即可。

## GitHub 帳號切換

安裝了 [GitHub CLI (gh)](https://cli.github.com) 後,小工具中會出現 GITHUB 區塊。新增帳號用小工具中的「＋ 新增帳號」按鈕 — 會顯示網址與一次性代碼,在瀏覽器中輸入即可(終端機 `gh auth login` 也仍然可用)。之後就能在小工具中切換 — 內部走與 `gh auth switch` 相同的通道,並在每次切換時執行 `gh auth setup-git`,讓 git push/pull(HTTPS)跟隨作用中帳號。權杖由 gh 保存在 keyring 中,小工具不會接觸。

已知限制:

- SSH 遠端(`git@github.com:...`)由 SSH 金鑰決定身分,不受此切換影響。僅 HTTPS 遠端有效。
- 提交作者(`git config user.name/email`)不會改變 — 切換後提交仍保留原有名字。
- VS Code、Copilot 等其他應用程式的 GitHub 工作階段使用各自的權杖,不會跟隨。
- 使用 SAML SSO 的組織儲存庫需要各帳號完成 SSO 授權才能存取。
- 新增帳號·每次切換執行的 `gh auth setup-git` 會在全域 git 設定中為 github.com 永久註冊 gh 作為 credential helper,取代既有的 GCM 設定 — 復原:`git config --global --unset-all credential.https://github.com.helper`。

## 技術

Tauri 2 + Rust,前端為 vanilla TypeScript。帳號切換、用量查詢與隔離登入全部在 Rust 端處理。
權杖不會進入 WebView。
CLI 的登入畫面透過虛擬主控台(PTY)讀取。

## 從原始碼建置

若不想直接下載,而想從原始碼自行建置,需要 [Node.js](https://nodejs.org) 與 [Rust](https://rustup.rs) 工具鏈。

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm run setup
```

`npm run setup` 會一次完成相依套件安裝與 App 建置。它不會傾倒冗長的記錄,只顯示載入指示與經過時間。

第一次會完整編譯 Rust,因此**可能需要 5~10 分鐘。**這不是載入卡住了,耐心等候即可。產出物在 Windows 為 `src-tauri\target\release\switcher.exe`,macOS 為 `src-tauri/target/release/bundle/macos/switcher.app` — 也可以把 App 移到應用程式資料夾。

開發模式執行請用 `npm run tauri dev`。

---

<div align="center">
<sub>Licensed under the <a href="../LICENSE">MIT License</a> — free for any use, including commercial. Keep the copyright and license notice.</sub>
</div>
