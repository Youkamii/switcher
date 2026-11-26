// 다른 PC로 옮기기 (#189·#192) — 구독 계정 로그인을 잠가 다른 PC에 넣는다.
// 받는 PC만 열 수 있다(#192): 받는 PC가 먼저 「받을 준비」로 받는 코드를 만들고, 보내는 PC는 그 코드와
// 비밀번호로 잠근다. 여는 열쇠는 받는 PC의 러스트 메모리에만 있고 10분 뒤나 한 번 가져온 뒤에 지워진다.
// 받는 코드는 공개 값이라 화면에 보여도 된다. 비밀번호만으로 잠그던 방식으로 되돌리지 말 것.
// 나르는 방법은 둘 다 있다: 복사해서 붙여넣는 글, 그 글을 담은 파일.
// 잠그기·풀기·10분 검사·저장은 전부 러스트(transfer.rs)가 한다. 화면은 코드·비밀번호·글만 건넨다.
// 받는 쪽은 전부 덮어쓴다(사용자 결정). 설정·화면 설정·메모는 옮기지 않는다 — 다시 넣지 말 것.
// 새 방법을 넣는다고 있던 방법을 없애지 말 것(복사를 넣으며 파일을 빼려다 지적받았다).
// 비밀번호를 변수에 오래 들고 있지 말 것 — 입력칸은 창이 닫힐 때 dialog()가 비운다.
import { invoke } from "@tauri-apps/api/core";
import { currentLang } from "./i18n";
import { dialog, dialogIsOpen, type Hooks } from "./dialog";

const messages = {
  ko: {
    open: "다른 PC로 옮기기", title: "다른 PC로 옮기기", exportTab: "내보내기", importTab: "가져오기",
    exportHelp: "구독 계정 로그인을 받는 PC만 열 수 있게 잠급니다. 먼저 받는 PC의 가져오기에서 「받을 준비」를 눌러 나온 코드를 여기에 넣으세요. 설정과 메모는 담지 않습니다.",
    code: "받는 PC의 코드", codeHint: "받는 PC에 보이는 SWR1.… 코드를 붙여넣으세요",
    password: "비밀번호 (8자 이상)", again: "비밀번호 한 번 더", mismatch: "두 비밀번호가 다릅니다", make: "만들기", making: "잠그는 중…",
    made: "만들었습니다", counts: "계정 {a}개 · 글 {c}자", until: "받는 코드를 만든 PC에서만, 그 코드가 살아 있는 10분 안에 한 번만 열립니다.",
    copy: "복사", copied: "복사했습니다. 받는 PC의 가져오기에 붙여넣으세요.", copyFailed: "복사하지 못했습니다. 아래 글을 눌러 전체를 직접 복사하세요.",
    saveFile: "파일로 저장", saved: "다운로드 폴더에 저장했습니다", reveal: "폴더에서 보기", textLabel: "복사해 옮기는 글",
    forget: "글과 비밀번호는 서로 다른 길로 보내세요. 이 PC에서도 다시 열 수 없습니다.",
    ready: "받을 준비", readyAgain: "새 코드 만들기", readyHelp: "1. 「받을 준비」를 눌러 나온 코드를 보내는 PC에 넣으세요. 코드는 남이 봐도 됩니다.",
    codeLabel: "이 PC의 받는 코드", codeUntil: "{t}까지 받을 수 있습니다. 한 번 가져오면 코드는 사라지고, 위젯을 꺼도 사라집니다.",
    codeCopied: "코드를 복사했습니다. 보내는 PC의 내보내기에 넣으세요.", codeCopyFailed: "복사하지 못했습니다. 코드를 눌러 직접 복사하세요.",
    importHelp: "2. 보내는 PC에서 만든 글이나 파일, 그리고 그때 정한 비밀번호를 넣으세요. 같은 계정의 로그인은 받은 것으로 덮어씁니다. 받은 것에 없는 계정은 그대로 둡니다.",
    shared: "같은 구독 로그인을 두 PC에서 함께 쓰면 한쪽이 다시 로그인해야 할 수 있습니다.",
    paste: "복사한 글 붙여넣기", pasteHint: "보내는 PC에서 복사한 글을 여기에 붙여넣으세요",
    file: "또는 받은 파일", pick: "또는 파일 고르기", notOurs: "옮기기 파일이 아닙니다",
    filePassword: "비밀번호", bring: "가져오기", bringing: "푸는 중…", noSource: "복사한 글을 붙여넣거나 파일을 고르세요",
    done: "가져왔습니다", added: "계정 {a}개 추가 · {r}개 덮어씀", live: "지금 로그인된 계정 {n}개는 로그인도 바꿨습니다.",
    rejected: "형식이 맞지 않아 넣지 않은 항목 {n}개", close: "닫기",
  },
  en: {
    open: "Move to another PC", title: "Move to another PC", exportTab: "Export", importTab: "Import",
    exportHelp: "Locks your subscription logins so that only the receiving PC can open them. First press “Get ready to receive” under Import on the receiving PC and enter the code it shows here. Settings and notes are not included.",
    code: "Code from the receiving PC", codeHint: "Paste the SWR1.… code shown on the receiving PC",
    password: "Password (8+ characters)", again: "Password again", mismatch: "The passwords do not match", make: "Create", making: "Locking…",
    made: "Created", counts: "{a} accounts · {c} characters", until: "Opens only on the PC that made the receive code, once, within the 10 minutes that code lives.",
    copy: "Copy", copied: "Copied. Paste it into Import on the other PC.", copyFailed: "Could not copy. Click the text below and copy all of it yourself.",
    saveFile: "Save as file", saved: "Saved to your Downloads folder", reveal: "Show in folder", textLabel: "Text to copy to the other PC",
    forget: "Send the text and the password by different routes. Even this PC cannot open it again.",
    ready: "Get ready to receive", readyAgain: "Make a new code", readyHelp: "1. Press “Get ready to receive” and enter the code on the sending PC. The code may be seen by others.",
    codeLabel: "This PC's receive code", codeUntil: "You can receive until {t}. The code is gone after one import, or when the widget quits.",
    codeCopied: "Code copied. Enter it under Export on the sending PC.", codeCopyFailed: "Could not copy. Click the code and copy it yourself.",
    importHelp: "2. Enter the text or file made on the other PC, and the password set there. Logins of the same account are overwritten with what you bring. Accounts that are not in it are kept.",
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

type ExportResult = { text: string; chars: number; accounts: number };
type ReceiveCode = { code: string; expires_at: number; seconds_left: number };
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
async function copyOut(value: string, source: HTMLTextAreaElement | HTMLInputElement): Promise<boolean> {
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

  // 내보내기: 받는 PC의 코드 + 비밀번호 → 잠긴 글 → 복사하거나 파일로 저장
  const exportForm = document.createElement("form"); exportForm.className = "api-form transfer-export";
  const code = document.createElement("input"); code.type = "text"; code.name = "transfer-code"; code.required = true; code.maxLength = 400;
  code.className = "transfer-code"; code.autocomplete = "off"; code.spellcheck = false; code.placeholder = t.codeHint;
  const first = passwordInput("transfer-password"), second = passwordInput("transfer-password-again");
  const exportError = line("", "api-error"); exportError.setAttribute("role", "alert");
  const make = button(t.make, "primary"); make.type = "submit";
  const exportResult = document.createElement("div"); exportResult.className = "transfer-result";
  exportForm.append(line(t.exportHelp), labelled(t.code, code), labelled(t.password, first), labelled(t.again, second), exportError, make, exportResult, line(t.forget));
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
      line(t.until, "transfer-until"),
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
      const done = await invoke<ExportResult>("transfer_export", { password, code: code.value });
      first.value = ""; second.value = ""; code.value = "";
      showExported(done);
    } catch (error) { exportError.textContent = String(error); }
    finally { busy(exportForm, make, false, t.make); hooks.fit(); }
  };

  // 가져오기: 받을 준비(받는 코드) → 붙여넣은 글이 먼저, 없으면 고른 파일, 그것도 없으면 찾아 둔 파일
  const importForm = document.createElement("form"); importForm.className = "api-form transfer-import";
  const ready = button(t.ready, "primary transfer-ready");
  const receive = document.createElement("div"); receive.className = "transfer-receive";
  const showCode = (state: ReceiveCode | null) => {
    ready.textContent = state ? t.readyAgain : t.ready;
    ready.classList.toggle("primary", !state);
    if (!state) { receive.replaceChildren(); return; }
    const mine = document.createElement("textarea"); mine.rows = 2; mine.readOnly = true; mine.value = state.code;
    mine.setAttribute("aria-label", t.codeLabel);
    mine.className = "transfer-code transfer-mine"; mine.spellcheck = false;
    mine.onfocus = () => mine.select();
    const note = line("", "transfer-note"); note.setAttribute("role", "status");
    const copy = button(t.copy, "primary transfer-code-copy");
    copy.onclick = async () => {
      const copied = await copyOut(state.code, mine);
      note.textContent = copied ? t.codeCopied : t.codeCopyFailed;
      note.classList.toggle("transfer-warn", !copied);
      hooks.fit();
    };
    const row = document.createElement("div"); row.className = "transfer-code-row"; row.append(mine, copy);
    receive.replaceChildren(labelled(t.codeLabel, row), line(fill(t.codeUntil, { t: clock(state.expires_at) }), "transfer-until"), note);
  };
  const loadCode = async () => {
    showCode(await invoke<ReceiveCode | null>("transfer_receive_state").catch(() => null));
    hooks.fit();
  };
  ready.onclick = async () => {
    if (working) return;
    ready.disabled = true;
    try { showCode(await invoke<ReceiveCode>("transfer_receive_start")); importError.textContent = ""; }
    catch (error) { importError.textContent = String(error); }
    finally { ready.disabled = false; hooks.fit(); }
  };
  const paste = document.createElement("textarea"); paste.className = "transfer-paste"; paste.name = "transfer-text"; paste.rows = 3;
  paste.spellcheck = false; paste.placeholder = t.pasteHint;
  const found = document.createElement("select"); found.name = "transfer-file";
  const foundLabel = labelled(t.file, found); foundLabel.hidden = true;
  const picker = document.createElement("input"); picker.type = "file"; picker.accept = ".swx"; picker.className = "transfer-picker";
  const filePassword = passwordInput("transfer-file-password"); filePassword.minLength = 1; filePassword.autocomplete = "off";
  const importError = line("", "api-error"); importError.setAttribute("role", "alert");
  const bring = button(t.bring, "primary"); bring.type = "submit";
  const importResult = document.createElement("div"); importResult.className = "transfer-result"; importResult.setAttribute("role", "status");
  importForm.append(line(t.readyHelp), ready, receive, line(t.importHelp), line(t.shared), labelled(t.paste, paste), foundLabel, labelled(t.pick, picker), labelled(t.filePassword, filePassword), importError, bring, importResult);
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
      // 한 번 가져오면 러스트가 열쇠를 지운다 — 화면의 코드도 지운다
      showCode(null);
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
    if (which === "import") { void loadCandidates(); void loadCode(); }
    hooks.fit();
    (which === "export" ? code : paste).focus();
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
