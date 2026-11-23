// 다른 PC로 옮기기 (#189) — 계정·API 키·메모·설정을 비밀번호로 잠근 파일 하나로 내보내고 가져온다.
// 잠그기·풀기와 저장은 전부 러스트(transfer.rs)가 한다. 화면은 비밀번호와 파일만 건넨다.
// 비밀번호를 변수에 오래 들고 있지 말 것 — 입력칸은 창이 닫힐 때 dialog()가 비운다.
import { invoke } from "@tauri-apps/api/core";
import { currentLang } from "./i18n";
import { apiIsBusy, dialog, type Hooks } from "./apiConnections";

const messages = {
  ko: {
    open: "다른 PC로 옮기기", title: "다른 PC로 옮기기", exportTab: "내보내기", importTab: "가져오기",
    exportHelp: "계정 로그인 정보, API 키, 메모, 설정을 파일 하나에 잠가 담습니다. 비밀번호를 모르면 열 수 없습니다.",
    password: "비밀번호 (8자 이상)", again: "비밀번호 한 번 더", mismatch: "두 비밀번호가 다릅니다", make: "파일 만들기", making: "잠그는 중…",
    made: "다운로드 폴더에 만들었습니다", counts: "계정 {a}개 · API 키 {k}개", reveal: "폴더에서 보기",
    forget: "비밀번호를 잊으면 이 파일은 열 수 없습니다. 파일은 옮긴 뒤 지우세요.",
    importHelp: "다른 PC에서 만든 파일과 그때 정한 비밀번호를 넣으세요. 지금 로그인된 CLI 상태는 바뀌지 않습니다.",
    file: "파일", pick: "다른 파일 고르기", none: "다운로드·바탕화면에서 옮기기 파일(.swx)을 찾지 못했습니다. 파일을 고르세요.",
    filePassword: "파일의 비밀번호", bring: "가져오기", bringing: "푸는 중…", noFile: "파일을 고르세요",
    done: "가져왔습니다", added: "계정 {a}개 추가 · {r}개 갱신", keys: "API 키 {a}개 추가 · {r}개 갱신",
    skipped: "넣지 않은 계정(같은 이름이 다른 계정의 것이거나 이미 다른 이름으로 있음): {n}",
    rejected: "형식이 맞지 않아 넣지 않은 항목 {n}개", view: "보기 설정은 위젯을 다시 띄우면 적용됩니다.", close: "닫기",
  },
  en: {
    open: "Move to another PC", title: "Move to another PC", exportTab: "Export", importTab: "Import",
    exportHelp: "Locks your account logins, API keys, notes and settings into one file. It cannot be opened without the password.",
    password: "Password (8+ characters)", again: "Password again", mismatch: "The passwords do not match", make: "Create file", making: "Locking…",
    made: "Created in your Downloads folder", counts: "{a} accounts · {k} API keys", reveal: "Show in folder",
    forget: "If you forget the password, the file cannot be opened. Delete the file after moving it.",
    importHelp: "Choose the file made on the other PC and enter its password. The CLI login on this PC is not changed.",
    file: "File", pick: "Choose another file", none: "No transfer file (.swx) found in Downloads or Desktop. Choose a file.",
    filePassword: "File password", bring: "Import", bringing: "Unlocking…", noFile: "Choose a file",
    done: "Imported", added: "{a} accounts added · {r} updated", keys: "{a} API keys added · {r} updated",
    skipped: "Accounts not imported (the name belongs to another account, or the account exists under another name): {n}",
    rejected: "{n} malformed items were not imported", view: "View settings apply the next time the widget starts.", close: "Close",
  },
};
const text = () => (currentLang() === "ko" ? messages.ko : messages.en);
const fill = (template: string, values: Record<string, string | number>) =>
  template.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key] ?? ""));

type ExportResult = { path: string; accounts: number; api_keys: number };
type ImportResult = {
  accounts_added: number; accounts_replaced: number; accounts_skipped: string[];
  api_keys_added: number; api_keys_replaced: number; rejected: number; ui: Record<string, string>;
};
type Candidate = { path: string; name: string; modified: number };

/// 함께 옮기는 화면 설정 — 이 목록의 키만 내보내고, 받을 때도 이 목록의 키만 쓴다
const UI_KEYS = ["switcher.viewmode", "switcher.order", "switcher.alpha", "switcher.privacy", "switcher.tools", "switcher.locked", "switcher.edgeanchor", "switcher.edgeside", "switcher.edgetab"];

function collectUi(): Record<string, string> {
  const ui: Record<string, string> = {};
  for (const key of UI_KEYS) {
    const value = localStorage.getItem(key);
    if (value !== null && value.length <= 4096) ui[key] = value;
  }
  return ui;
}

function passwordInput(name: string): HTMLInputElement {
  const input = document.createElement("input");
  input.type = "password"; input.name = name; input.required = true; input.minLength = 8; input.maxLength = 256;
  input.autocomplete = "new-password"; input.spellcheck = false;
  return input;
}
function labelled(label: string, control: HTMLElement): HTMLLabelElement {
  const host = document.createElement("label"); host.textContent = label; host.append(control); return host;
}
function line(content: string, className = "help"): HTMLParagraphElement {
  const p = document.createElement("p"); p.className = className; p.textContent = content; return p;
}

async function fileBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  return btoa(binary);
}

export function openTransfer(hooks: Hooks) {
  if (apiIsBusy()) return;
  const t = text();
  let working = false;
  const host = dialog(hooks, t.title, () => !working);
  host.classList.add("transfer-dialog");
  const tabs = document.createElement("div"); tabs.className = "transfer-tabs"; tabs.setAttribute("role", "tablist");
  const exportTab = document.createElement("button"); exportTab.type = "button"; exportTab.textContent = t.exportTab; exportTab.setAttribute("role", "tab");
  const importTab = document.createElement("button"); importTab.type = "button"; importTab.textContent = t.importTab; importTab.setAttribute("role", "tab");
  tabs.append(exportTab, importTab);
  const busy = (form: HTMLFormElement, button: HTMLButtonElement, on: boolean, label: string) => {
    working = on; button.disabled = on; button.textContent = label;
    host.querySelector<HTMLButtonElement>(".api-close")!.disabled = on;
    exportTab.disabled = on; importTab.disabled = on;
    form.querySelectorAll<HTMLInputElement>("input, select").forEach(control => { control.disabled = on; });
  };

  // 내보내기
  const exportForm = document.createElement("form"); exportForm.className = "api-form transfer-export";
  const first = passwordInput("transfer-password"), second = passwordInput("transfer-password-again");
  const exportError = line("", "api-error"); exportError.setAttribute("role", "alert");
  const make = document.createElement("button"); make.type = "submit"; make.className = "primary"; make.textContent = t.make;
  const exportResult = document.createElement("div"); exportResult.className = "transfer-result"; exportResult.setAttribute("role", "status");
  exportForm.append(line(t.exportHelp), labelled(t.password, first), labelled(t.again, second), exportError, make, line(t.forget), exportResult);
  exportForm.onsubmit = async event => {
    event.preventDefault();
    if (working) return;
    exportError.textContent = ""; exportResult.replaceChildren();
    if (first.value !== second.value) { exportError.textContent = t.mismatch; hooks.fit(); return; }
    const password = first.value;
    busy(exportForm, make, true, t.making);
    try {
      const done = await invoke<ExportResult>("transfer_export", { password, ui: collectUi() });
      first.value = ""; second.value = "";
      const name = done.path.split(/[\\/]/).pop() ?? done.path;
      const reveal = document.createElement("button"); reveal.type = "button"; reveal.className = "transfer-reveal"; reveal.textContent = t.reveal;
      reveal.onclick = () => { void invoke("transfer_reveal", { path: done.path }).catch(error => hooks.toast(String(error), true)); };
      exportResult.append(line(t.made, "transfer-done"), line(name, "transfer-file"), line(fill(t.counts, { a: done.accounts, k: done.api_keys })), reveal);
    } catch (error) { exportError.textContent = String(error); }
    finally { busy(exportForm, make, false, t.make); hooks.fit(); }
  };

  // 가져오기
  const importForm = document.createElement("form"); importForm.className = "api-form transfer-import";
  const found = document.createElement("select"); found.name = "transfer-file";
  const picker = document.createElement("input"); picker.type = "file"; picker.accept = ".swx"; picker.className = "transfer-picker";
  const pickLabel = labelled(t.pick, picker);
  const none = line(t.none); none.hidden = true;
  const foundLabel = labelled(t.file, found);
  const filePassword = passwordInput("transfer-file-password"); filePassword.minLength = 1; filePassword.autocomplete = "off";
  const importError = line("", "api-error"); importError.setAttribute("role", "alert");
  const bring = document.createElement("button"); bring.type = "submit"; bring.className = "primary"; bring.textContent = t.bring;
  const importResult = document.createElement("div"); importResult.className = "transfer-result"; importResult.setAttribute("role", "status");
  importForm.append(line(t.importHelp), foundLabel, none, pickLabel, labelled(t.filePassword, filePassword), importError, bring, importResult);
  const loadCandidates = async () => {
    const list = await invoke<Candidate[]>("transfer_candidates").catch(() => [] as Candidate[]);
    found.replaceChildren(...list.map(item => new Option(item.name, item.path)));
    foundLabel.hidden = list.length === 0; none.hidden = list.length > 0;
    hooks.fit();
  };
  importForm.onsubmit = async event => {
    event.preventDefault();
    if (working) return;
    importError.textContent = ""; importResult.replaceChildren();
    const chosen = picker.files?.[0] ?? null;
    if (!chosen && !found.value) { importError.textContent = t.noFile; hooks.fit(); return; }
    const password = filePassword.value;
    const path = chosen ? null : found.value;
    busy(importForm, bring, true, t.bringing);
    try {
      const data = chosen ? await fileBase64(chosen) : null;
      const done = await invoke<ImportResult>("transfer_import", { password, path, data });
      filePassword.value = "";
      // 화면 설정은 우리가 아는 키만, 다음 실행부터 — 지금 화면을 뒤집지 않는다
      for (const key of UI_KEYS) {
        const value = done.ui[key];
        if (typeof value === "string" && value.length <= 4096) { try { localStorage.setItem(key, value); } catch { /* 저장소가 막혀 있어도 계정·키는 들어왔다 */ } }
      }
      importResult.append(
        line(t.done, "transfer-done"),
        line(fill(t.added, { a: done.accounts_added, r: done.accounts_replaced })),
        line(fill(t.keys, { a: done.api_keys_added, r: done.api_keys_replaced })),
      );
      if (done.accounts_skipped.length) importResult.append(line(fill(t.skipped, { n: done.accounts_skipped.join(", ") }), "transfer-warn"));
      if (done.rejected) importResult.append(line(fill(t.rejected, { n: done.rejected }), "transfer-warn"));
      importResult.append(line(t.view));
    } catch (error) { importError.textContent = String(error); }
    finally { busy(importForm, bring, false, t.bring); hooks.fit(); }
  };

  const show = (which: "export" | "import") => {
    if (working) return;
    exportForm.hidden = which !== "export"; importForm.hidden = which !== "import";
    exportTab.setAttribute("aria-selected", String(which === "export")); importTab.setAttribute("aria-selected", String(which === "import"));
    if (which === "import") void loadCandidates();
    hooks.fit();
    (which === "export" ? first : filePassword).focus();
  };
  exportTab.onclick = () => show("export");
  importTab.onclick = () => show("import");
  host.append(tabs, exportForm, importForm);
  host.showModal();
  show("export");
}

/// 일반 모드 맨 아래의 한 줄 — 옮기기 창을 연다
export function transferRow(hooks: Hooks): HTMLElement {
  const row = document.createElement("div"); row.className = "transfer-row";
  const open = document.createElement("button"); open.type = "button"; open.className = "transfer-open";
  open.textContent = text().open; open.dataset.apiFocus = "transfer/open";
  open.onclick = () => openTransfer(hooks);
  row.append(open);
  return row;
}
