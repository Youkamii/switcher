// 다른 PC로 옮기기 (#189) — 구독 계정 로그인을 비밀번호로 잠가 다른 PC에 넣는다.
// 나르는 방법은 둘 다 있다: 복사해서 붙여넣는 글, 그 글을 담은 파일. 만든 지 10분이 지나면 못 쓴다.
// 잠그기·풀기·10분 검사·저장은 전부 러스트(transfer.rs)가 한다. 화면은 비밀번호와 글만 건넨다.
// 받는 쪽은 전부 덮어쓴다(사용자 결정). 설정·화면 설정·메모는 옮기지 않는다 — 다시 넣지 말 것.
// 새 방법을 넣는다고 있던 방법을 없애지 말 것(복사를 넣으며 파일을 빼려다 지적받았다).
// 비밀번호를 변수에 오래 들고 있지 말 것 — 입력칸은 창이 닫힐 때 dialog()가 비운다.
import { invoke } from "@tauri-apps/api/core";
import { currentLang } from "./i18n";
import { dialog, dialogIsOpen, type Hooks } from "./dialog";

const messages = {
  ko: {
    open: "다른 PC로 옮기기", title: "다른 PC로 옮기기", exportTab: "내보내기", importTab: "가져오기",
    exportHelp: "구독 계정 로그인을 비밀번호로 잠급니다. 복사해서 붙여넣거나 파일로 옮기면 되고, 만든 지 10분 안에만 가져올 수 있습니다. 설정과 메모는 담지 않습니다.",
    password: "비밀번호 (8자 이상)", again: "비밀번호 한 번 더", mismatch: "두 비밀번호가 다릅니다", make: "만들기", making: "잠그는 중…",
    made: "만들었습니다", counts: "계정 {a}개 · 글 {c}자", until: "{t}까지, 10분 동안만 가져올 수 있습니다.",
    copy: "복사", copied: "복사했습니다. 받는 PC의 가져오기에 붙여넣으세요.", copyFailed: "복사하지 못했습니다. 아래 글을 눌러 전체를 직접 복사하세요.",
    saveFile: "파일로 저장", saved: "다운로드 폴더에 저장했습니다", reveal: "폴더에서 보기", textLabel: "복사해 옮기는 글",
    forget: "비밀번호를 모르면 열 수 없습니다. 파일로 저장했으면 옮긴 뒤 지우세요.",
    importHelp: "보내는 PC에서 만든 글이나 파일, 그리고 그때 정한 비밀번호를 넣으세요. 같은 계정의 로그인은 받은 것으로 덮어씁니다. 받은 것에 없는 계정은 그대로 둡니다.",
    shared: "같은 구독 로그인을 두 PC에서 함께 쓰면 한쪽이 다시 로그인해야 할 수 있습니다.",
    paste: "복사한 글 붙여넣기", pasteHint: "보내는 PC에서 복사한 글을 여기에 붙여넣으세요",
    file: "또는 받은 파일", pick: "또는 파일 고르기", notOurs: "옮기기 파일이 아닙니다",
    filePassword: "비밀번호", bring: "가져오기", bringing: "푸는 중…", noSource: "복사한 글을 붙여넣거나 파일을 고르세요",
    done: "가져왔습니다", added: "계정 {a}개 추가 · {r}개 덮어씀", live: "지금 로그인된 계정 {n}개는 로그인도 바꿨습니다.",
    rejected: "형식이 맞지 않아 넣지 않은 항목 {n}개", close: "닫기",
  },
  en: {
    open: "Move to another PC", title: "Move to another PC", exportTab: "Export", importTab: "Import",
    exportHelp: "Locks your subscription logins with a password. Copy and paste the result or move it as a file; it can be imported only within 10 minutes of being created. Settings and notes are not included.",
    password: "Password (8+ characters)", again: "Password again", mismatch: "The passwords do not match", make: "Create", making: "Locking…",
    made: "Created", counts: "{a} accounts · {c} characters", until: "Can be imported for 10 minutes, until {t}.",
    copy: "Copy", copied: "Copied. Paste it into Import on the other PC.", copyFailed: "Could not copy. Click the text below and copy all of it yourself.",
    saveFile: "Save as file", saved: "Saved to your Downloads folder", reveal: "Show in folder", textLabel: "Text to copy to the other PC",
    forget: "It cannot be opened without the password. If you saved a file, delete it after moving it.",
    importHelp: "Enter the text or file made on the other PC, and the password set there. Logins of the same account are overwritten with what you bring. Accounts that are not in it are kept.",
    shared: "Using the same subscription login on two PCs may require signing in again on one of them.",
    paste: "Paste the copied text", pasteHint: "Paste the text copied on the other PC here",
    file: "Or a file found here", pick: "Or choose a file", notOurs: "Not a transfer file",
    filePassword: "Password", bring: "Import", bringing: "Unlocking…", noSource: "Paste the copied text or choose a file",
    done: "Imported", added: "{a} accounts added · {r} overwritten", live: "{n} of them are logged in here now; that login was replaced too.",
    rejected: "{n} malformed items were not imported", close: "Close",
  },
};
const text = () => (currentLang() === "ko" ? messages.ko : messages.en);
const fill = (template: string, values: Record<string, string | number>) =>
  template.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key] ?? ""));

type ExportResult = { text: string; chars: number; accounts: number; expires_at: number };
type SavedFile = { path: string; name: string };
type ImportResult = {
  accounts_added: number; accounts_replaced: number; live_replaced: number;
  rejected: number; problems: string[];
};
type Candidate = { path: string; name: string; modified: number };

/// 파일을 읽어 넘기는 크기의 상한 — 러스트의 TEXT_MAX와 같다
const FILE_MAX = 8 * 1024 * 1024;

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
function button(label: string, className: string): HTMLButtonElement {
  const control = document.createElement("button"); control.type = "button"; control.className = className; control.textContent = label; return control;
}

/// 클립보드에 넣는다. 웹뷰가 클립보드를 막으면 글 상자를 전체 선택해 복사한다.
async function copyOut(value: string, source: HTMLTextAreaElement): Promise<boolean> {
  try { await navigator.clipboard.writeText(value); return true; } catch { /* 아래 길로 */ }
  source.focus(); source.select();
  try { return document.execCommand("copy"); } catch { return false; }
}

function clock(seconds: number): string {
  return new Date(seconds * 1000).toLocaleTimeString(currentLang() === "ko" ? "ko-KR" : "en-US", { hour: "numeric", minute: "2-digit" });
}

export function openTransfer(hooks: Hooks) {
  if (dialogIsOpen()) return;
  const t = text();
  let working = false;
  const host = dialog(hooks, t.title, () => !working);
  host.classList.add("transfer-dialog");
  const tabs = document.createElement("div"); tabs.className = "transfer-tabs"; tabs.setAttribute("role", "tablist");
  const exportTab = button(t.exportTab, ""); exportTab.setAttribute("role", "tab");
  const importTab = button(t.importTab, ""); importTab.setAttribute("role", "tab");
  tabs.append(exportTab, importTab);
  const busy = (form: HTMLFormElement, submit: HTMLButtonElement, on: boolean, label: string) => {
    working = on; submit.disabled = on; submit.textContent = label;
    host.querySelector<HTMLButtonElement>(".api-close")!.disabled = on;
    exportTab.disabled = on; importTab.disabled = on;
    form.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("input, select, textarea").forEach(control => { control.disabled = on; });
  };

  // 내보내기: 비밀번호 → 잠긴 글 → 복사하거나 파일로 저장
  const exportForm = document.createElement("form"); exportForm.className = "api-form transfer-export";
  const first = passwordInput("transfer-password"), second = passwordInput("transfer-password-again");
  const exportError = line("", "api-error"); exportError.setAttribute("role", "alert");
  const make = button(t.make, "primary"); make.type = "submit";
  const exportResult = document.createElement("div"); exportResult.className = "transfer-result";
  exportForm.append(line(t.exportHelp), labelled(t.password, first), labelled(t.again, second), exportError, make, exportResult, line(t.forget));
  const showExported = (done: ExportResult) => {
    const box = document.createElement("textarea"); box.className = "transfer-text"; box.readOnly = true; box.rows = 2; box.spellcheck = false;
    box.value = done.text; box.setAttribute("aria-label", t.textLabel);
    box.onfocus = () => box.select();
    const note = line("", "transfer-note"); note.setAttribute("role", "status");
    const copy = button(t.copy, "primary transfer-copy");
    copy.onclick = async () => {
      const copied = await copyOut(done.text, box);
      note.textContent = copied ? t.copied : t.copyFailed;
      note.classList.toggle("transfer-warn", !copied);
      hooks.fit();
    };
    const save = button(t.saveFile, "transfer-save");
    save.onclick = async () => {
      save.disabled = true;
      note.classList.remove("transfer-warn");
      try {
        const file = await invoke<SavedFile>("transfer_save", { text: done.text });
        const reveal = button(t.reveal, "transfer-reveal");
        reveal.onclick = () => { void invoke("transfer_reveal", { path: file.path }).catch(error => hooks.toast(String(error), true)); };
        const name = document.createElement("span"); name.className = "transfer-file"; name.textContent = file.name;
        note.replaceChildren(`${t.saved}: `, name, " ", reveal);
      } catch (error) { note.textContent = String(error); note.classList.add("transfer-warn"); }
      finally { save.disabled = false; hooks.fit(); }
    };
    const actions = document.createElement("div"); actions.className = "transfer-actions"; actions.append(copy, save);
    exportResult.replaceChildren(
      line(t.made, "transfer-done"),
      line(fill(t.counts, { a: done.accounts, c: done.chars.toLocaleString() })),
      line(fill(t.until, { t: clock(done.expires_at) }), "transfer-until"),
      actions, box, note,
    );
  };
  exportForm.onsubmit = async event => {
    event.preventDefault();
    if (working) return;
    exportError.textContent = ""; exportResult.replaceChildren();
    if (first.value !== second.value) { exportError.textContent = t.mismatch; hooks.fit(); return; }
    const password = first.value;
    busy(exportForm, make, true, t.making);
    try {
      const done = await invoke<ExportResult>("transfer_export", { password });
      first.value = ""; second.value = "";
      showExported(done);
    } catch (error) { exportError.textContent = String(error); }
    finally { busy(exportForm, make, false, t.make); hooks.fit(); }
  };

  // 가져오기: 붙여넣은 글이 먼저, 없으면 고른 파일, 그것도 없으면 찾아 둔 파일
  const importForm = document.createElement("form"); importForm.className = "api-form transfer-import";
  const paste = document.createElement("textarea"); paste.className = "transfer-paste"; paste.name = "transfer-text"; paste.rows = 3;
  paste.spellcheck = false; paste.placeholder = t.pasteHint;
  const found = document.createElement("select"); found.name = "transfer-file";
  const foundLabel = labelled(t.file, found); foundLabel.hidden = true;
  const picker = document.createElement("input"); picker.type = "file"; picker.accept = ".swx"; picker.className = "transfer-picker";
  const filePassword = passwordInput("transfer-file-password"); filePassword.minLength = 1; filePassword.autocomplete = "off";
  const importError = line("", "api-error"); importError.setAttribute("role", "alert");
  const bring = button(t.bring, "primary"); bring.type = "submit";
  const importResult = document.createElement("div"); importResult.className = "transfer-result"; importResult.setAttribute("role", "status");
  importForm.append(line(t.importHelp), line(t.shared), labelled(t.paste, paste), foundLabel, labelled(t.pick, picker), labelled(t.filePassword, filePassword), importError, bring, importResult);
  const loadCandidates = async () => {
    const list = await invoke<Candidate[]>("transfer_candidates").catch(() => [] as Candidate[]);
    found.replaceChildren(...list.map(item => new Option(item.name, item.path)));
    foundLabel.hidden = list.length === 0;
    hooks.fit();
  };
  importForm.onsubmit = async event => {
    event.preventDefault();
    if (working) return;
    importError.textContent = ""; importResult.replaceChildren();
    const pasted = paste.value.trim();
    const chosen = picker.files?.[0] ?? null;
    const path = found.value || null;
    if (!pasted && !chosen && !path) { importError.textContent = t.noSource; hooks.fit(); return; }
    const password = filePassword.value;
    busy(importForm, bring, true, t.bringing);
    try {
      let content: string | null = pasted || null;
      if (!content && chosen) {
        if (chosen.size > FILE_MAX) throw t.notOurs;
        content = await chosen.text();
      }
      const done = await invoke<ImportResult>("transfer_import", { password, text: content, path: content ? null : path });
      filePassword.value = ""; paste.value = "";
      importResult.append(
        line(t.done, "transfer-done"),
        line(fill(t.added, { a: done.accounts_added, r: done.accounts_replaced })),
      );
      if (done.live_replaced) importResult.append(line(fill(t.live, { n: done.live_replaced })));
      for (const problem of done.problems) importResult.append(line(problem, "transfer-warn"));
      if (done.rejected) importResult.append(line(fill(t.rejected, { n: done.rejected }), "transfer-warn"));
    } catch (error) { importError.textContent = String(error); }
    finally { busy(importForm, bring, false, t.bring); hooks.fit(); }
  };

  const show = (which: "export" | "import") => {
    if (working) return;
    exportForm.hidden = which !== "export"; importForm.hidden = which !== "import";
    exportTab.setAttribute("aria-selected", String(which === "export")); importTab.setAttribute("aria-selected", String(which === "import"));
    if (which === "import") void loadCandidates();
    hooks.fit();
    (which === "export" ? first : paste).focus();
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
  const open = button(text().open, "transfer-open"); open.dataset.apiFocus = "transfer/open";
  open.onclick = () => openTransfer(hooks);
  row.append(open);
  return row;
}
