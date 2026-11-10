import { invoke } from "@tauri-apps/api/core";
import { currentLang } from "./i18n";
import { icon, iconButton, setButtonIcon } from "./uiIcons";

const messages = {
  ko: {
    title: "API 연결", add: "API 키 등록", name: "연결 이름", key: "API 키", save: "저장하고 확인", cancel: "닫기", replace: "키 교체",
    billing: "API는 구독과 별도 과금됩니다. 복사한 명령으로 시작한 CLI에서만 이 키를 사용합니다.",
    saved: "저장됨 · 확인 전", check: "연결 확인", checked: "연결 확인됨", checking: "연결 확인 중…", copy: "실행 명령 복사", copied: "명령을 복사했습니다. 프로젝트 터미널에 붙여넣으세요.",
    remove: "연결 삭제", confirm: "한 번 더 눌러 삭제", failed: "아래 명령을 직접 복사하세요", noQuota: "API · 별도 과금", console: "키 발급 페이지 열기 ↗",
    next: "프로젝트 터미널에서 시작", instruction: "작업할 프로젝트의 터미널에 이 명령을 붙여넣으세요. 다음에도 같은 명령을 사용할 수 있습니다.",
    checkNote: "모델 목록만 조회합니다. 생성 권한·잔액은 확인하지 않습니다.", keyNote: "키는 OS 보안 저장소에 보관하며 다시 표시하지 않습니다.",
    claudeNote: "Claude 키는 발급할 때 사용할 워크스페이스를 지정하세요.", invalid_key: "키 교체 필요", forbidden: "조회 권한 확인 필요", rate_limited: "요청 제한 · 다시 확인", unavailable: "연결 확인 필요",
    complete: "키 저장 완료", nameHint: "예: work · 영문, 숫자, - 또는 _", replacement: "새 키로 교체하면 다음 CLI 실행부터 적용됩니다.",
  },
  en: {
    title: "API connections", add: "Add API key", name: "Connection name", key: "API key", save: "Save and check", cancel: "Close", replace: "Replace key",
    billing: "API usage is billed separately. This key is used only by the CLI started with the copied command.",
    saved: "Saved · not checked", check: "Check connection", checked: "Connection checked", checking: "Checking connection…", copy: "Copy launch command", copied: "Command copied. Paste it into your project terminal.",
    remove: "Delete connection", confirm: "Click again to delete", failed: "Copy the command below", noQuota: "API · separate billing", console: "Open API key page ↗",
    next: "Start in your project terminal", instruction: "Paste this command into your project terminal. You can reuse the same command next time.",
    checkNote: "Checks the model list only, not generation access or balance.", keyNote: "Keys stay in the OS secure store and are never displayed again.",
    claudeNote: "Select a workspace when creating your Claude API key.", invalid_key: "Replace key", forbidden: "Check model permissions", rate_limited: "Rate limited · retry", unavailable: "Check connection",
    complete: "Key saved", nameHint: "e.g. work · letters, numbers, - or _", replacement: "The replacement key applies to the next CLI launch.",
  },
};
export function apiText() { return currentLang() === "ko" ? messages.ko : messages.en; }
let apiPending = 0;
export function apiIsBusy(): boolean { return apiPending > 0 || document.querySelector(".api-dialog") !== null; }
type CheckState = "unchecked" | "ready" | "invalid_key" | "forbidden" | "rate_limited" | "unavailable";
type Connection = { provider: string; name: string; checked_at?: number | null; check_state?: CheckState };
type Hooks = { changed: () => Promise<void>; fit: () => void; toast: (message: string, error?: boolean) => void };
function paragraph(content: string, className = "help") {
  const p = document.createElement("p"); p.className = className; p.textContent = content; return p;
}
function stateLabel(connection: Connection) {
  const text = apiText(), state = connection.check_state ?? "unchecked";
  return state === "ready" ? text.checked : state === "unchecked" ? text.saved : text[state];
}
function dialog(hooks: Hooks, title: string, canClose = () => true) {
  document.querySelector<HTMLDialogElement>(".api-dialog")?.dispatchEvent(new Event("api-dismiss"));
  const previousFocus = document.activeElement;
  const host = document.createElement("dialog"); host.className = "api-dialog";
  const head = document.createElement("div"); head.className = "api-dialog-head";
  const heading = document.createElement("h2"); heading.id = "api-dialog-title"; heading.textContent = title;
  host.setAttribute("aria-labelledby", heading.id);
  const close = iconButton("close", apiText().cancel, "api-close"); close.onclick = () => dismiss();
  head.append(heading, close); host.append(head);
  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return; cleaned = true;
    host.querySelectorAll<HTMLInputElement>('input[type="password"]').forEach(input => { input.value = ""; });
    host.remove(); hooks.fit();
    const focusId = previousFocus instanceof HTMLElement ? previousFocus.dataset.apiFocus : undefined;
    void hooks.changed().then(() => {
      const target = focusId ? document.querySelector<HTMLElement>(`[data-api-focus="${CSS.escape(focusId)}"]`) : previousFocus;
      if (target instanceof HTMLElement && target.isConnected && !document.querySelector(".api-dialog")) target.focus();
    });
  };
  const dismiss = () => { if (!canClose()) return; host.close(); cleanup(); };
  // Native dialog close events may wait for a rendering frame in inactive WebKit.
  // Clear secrets and release the renderer synchronously on the user's action.
  host.addEventListener("cancel", event => { event.preventDefault(); dismiss(); });
  host.addEventListener("api-dismiss", dismiss);
  host.addEventListener("close", cleanup, { once: true });
  document.body.append(host);
  return host;
}
async function launchPanel(connection: Connection, host: HTMLElement, hooks: Hooks) {
  const text = apiText();
  const block = document.createElement("div"); block.className = "api-launch";
  const heading = document.createElement("h3"); heading.textContent = text.next;
  block.append(heading, paragraph(text.instruction)); host.append(block);
  try {
    const command = await invoke<string>("api_launch_command", { provider: connection.provider, name: connection.name });
    if (!host.isConnected) return;
    const code = document.createElement("textarea"); code.readOnly = true; code.value = command;
    code.setAttribute("aria-label", text.copy); code.rows = 3;
    const copy = document.createElement("button"); copy.type = "button"; copy.className = "primary api-copy"; copy.append(icon("copy"), text.copy);
    const feedback = paragraph("", "api-feedback"); feedback.setAttribute("role", "status");
    copy.onclick = () => {
      // Keep a selectable command available even if inactive WKWebView clipboard permission stalls.
      code.focus(); code.select(); feedback.textContent = text.failed;
      void navigator.clipboard?.writeText(command).then(() => { if (feedback.isConnected) feedback.textContent = text.copied; }, () => {});
    };
    block.append(code, copy, feedback);
  } catch (error) { if (host.isConnected) block.append(paragraph(String(error), "api-error")); }
  hooks.fit();
}

export function openApiRegistration(hooks: Hooks, initialProvider = "claude", existing?: Connection) {
  if (apiIsBusy()) return;
  const text = apiText();
  let saving = false;
  const host = dialog(hooks, existing ? text.replace : text.add, () => !saving);
  let provider = existing?.provider ?? initialProvider;
  const form = document.createElement("form"); form.className = "api-form";
  const providers = document.createElement("div"); providers.className = "api-providers"; providers.setAttribute("aria-label", "API provider");
  const choices: HTMLButtonElement[] = [];
  const providerNote = paragraph(text.claudeNote);
  const key = document.createElement("input"); key.type = "password"; key.required = true; key.maxLength = 4096; key.minLength = 16; key.autocomplete = "new-password"; key.spellcheck = false; key.pattern = "sk-[!-~]+";
  for (const [value, label] of [["claude", "Claude"], ["codex", "OpenAI · Codex"]]) {
    const button = document.createElement("button"); button.type = "button"; button.textContent = label; button.dataset.provider = value;
    button.disabled = Boolean(existing); button.setAttribute("aria-pressed", String(value === provider));
    button.onclick = () => { provider = value; key.value = ""; choices.forEach(choice => choice.setAttribute("aria-pressed", String(choice === button))); providerNote.hidden = provider !== "claude"; hooks.fit(); };
    choices.push(button); providers.append(button);
  }
  providerNote.hidden = provider !== "claude";
  const consoleLink = document.createElement("button"); consoleLink.type = "button"; consoleLink.className = "api-console"; consoleLink.textContent = text.console;
  consoleLink.onclick = () => { void invoke("api_open_console", { provider }).catch(error => { errorLine.textContent = String(error); hooks.fit(); }); };
  const nameLabel = document.createElement("label"); nameLabel.textContent = text.name;
  const name = document.createElement("input"); name.name = "connection-name"; name.required = true; name.maxLength = 32; name.pattern = "[A-Za-z0-9_-]+"; name.autocomplete = "off";
  name.placeholder = text.nameHint; name.value = existing?.name ?? ""; name.readOnly = Boolean(existing); nameLabel.append(name);
  const keyLabel = document.createElement("label"); keyLabel.textContent = text.key; key.name = "api-key"; key.placeholder = "sk-…"; keyLabel.append(key);
  const errorLine = paragraph("", "api-error"); errorLine.setAttribute("role", "alert");
  const save = document.createElement("button"); save.type = "submit"; save.className = "primary api-save"; save.append(icon("shield"), text.save);
  form.append(providers, consoleLink, nameLabel, keyLabel, paragraph(existing ? text.replacement : text.keyNote), providerNote, errorLine, save);
  host.append(paragraph(text.billing, "api-billing"), form);
  // A save is a short local mutation. Network checks may continue after the dialog closes.
  form.onsubmit = async event => {
    event.preventDefault(); if (saving) return;
    saving = true; apiPending++; save.disabled = true; name.readOnly = true; key.readOnly = true; choices.forEach(b => { b.disabled = true; });
    const close = host.querySelector<HTMLButtonElement>(".api-close")!; close.disabled = true;
    const connection: Connection = { provider, name: name.value.trim().toLowerCase() };
    let stored = false;
    try {
      await invoke("api_save", { ...connection, key: key.value.trim(), replace: Boolean(existing) }); stored = true;
    } catch (error) { errorLine.textContent = String(error); }
    finally {
      key.value = ""; saving = false; apiPending--; close.disabled = false; save.disabled = false;
      name.readOnly = Boolean(existing); key.readOnly = false; choices.forEach(b => { b.disabled = Boolean(existing); });
    }
    if (!stored) { key.focus(); return; }
    form.remove(); host.querySelector("h2")!.textContent = text.complete;
    const status = paragraph(text.checking, "api-check-result"); status.setAttribute("role", "status"); host.append(status, paragraph(text.checkNote));
    void launchPanel(connection, host, hooks);
    hooks.fit();
    try { const checked = await invoke<Connection>("api_check", connection); if (host.isConnected) { status.textContent = stateLabel(checked); status.dataset.state = "ready"; } }
    catch (error) { if (host.isConnected) { status.textContent = String(error); status.dataset.state = "error"; } }
    finally { if (!host.isConnected) void hooks.changed(); hooks.fit(); }
  };
  host.showModal(); hooks.fit(); (existing ? key : name).focus();
}

export async function apiConnections(provider: string, hooks: Hooks): Promise<HTMLElement> {
  const text = apiText();
  const panel = document.createElement("div"); panel.className = "api-panel";
  const connections = await invoke<Connection[]>("api_list", { provider });
  panel.hidden = connections.length === 0;
  for (const connection of connections) {
    const card = document.createElement("div"); card.className = "api-card";
    const head = document.createElement("div"); head.className = "api-card-info";
    const name = document.createElement("strong"); name.className = "card-name"; name.textContent = connection.name;
    const badge = document.createElement("span"); badge.className = "api-badge"; badge.textContent = "API";
    const status = paragraph(stateLabel(connection), "api-status"); status.dataset.state = connection.check_state ?? "unchecked";
    status.setAttribute("role", "status");
    status.title = text.noQuota + (connection.checked_at ? " · " + new Date(connection.checked_at * 1000).toLocaleString() : "");
    head.append(name, badge, status);
    const actions = document.createElement("div"); actions.className = "api-actions";
    const launch = iconButton("terminal", text.copy, "api-run");
    launch.dataset.apiFocus = `${provider}/${connection.name}/run`;
    launch.onclick = () => { const host = dialog(hooks, `${connection.name} · API`); host.append(paragraph(text.billing, "api-billing")); host.showModal(); void launchPanel(connection, host, hooks); hooks.fit(); };
    const check = iconButton("reconnect", text.check, "api-check");
    check.onclick = async () => {
      check.disabled = true; status.textContent = text.checking;
      try { const checked = await invoke<Connection>("api_check", { provider, name: connection.name }); status.textContent = stateLabel(checked); status.dataset.state = "ready"; }
      catch (error) { status.textContent = String(error); status.dataset.state = "error"; }
      finally { check.disabled = false; hooks.fit(); }
    };
    const replace = iconButton("key", text.replace, "api-replace"); replace.dataset.apiFocus = `${provider}/${connection.name}/replace`; replace.onclick = () => openApiRegistration(hooks, provider, connection);
    const remove = iconButton("trash", text.remove, "api-delete");
    let armedAt: number | null = null;
    const disarm = () => { armedAt = null; setButtonIcon(remove, "trash", text.remove); remove.classList.remove("danger-armed"); document.removeEventListener("pointerdown", outside, true); window.removeEventListener("blur", disarm); };
    const outside = (event: Event) => { if (!remove.contains(event.target as Node)) disarm(); };
    card.addEventListener("pointerleave", disarm);
    card.addEventListener("focusout", event => { if (!card.contains(event.relatedTarget as Node | null)) disarm(); });
    remove.onclick = async () => {
      if (armedAt === null || Date.now() < armedAt || Date.now() - armedAt > 3000) {
        armedAt = Date.now(); setButtonIcon(remove, "check", text.confirm); remove.classList.add("danger-armed");
        document.addEventListener("pointerdown", outside, true); window.addEventListener("blur", disarm); return;
      }
      disarm(); remove.disabled = true; apiPending++;
      try { await invoke("api_delete", { provider, name: connection.name }); card.remove(); panel.hidden = !panel.childElementCount; }
      catch (error) { status.textContent = String(error); status.dataset.state = "error"; }
      finally { apiPending--; remove.disabled = false; hooks.fit(); }
    };
    actions.append(launch, check, replace, remove); card.append(head, actions); panel.append(card);
  }
  return panel;
}
