import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { currentLang } from "./i18n";
import { icon, iconButton, setButtonIcon } from "./uiIcons";

const messages = {
  ko: {
    title: "API 연결", add: "API 키 등록", name: "연결 이름", key: "API 키", save: "저장하고 사용", cancel: "닫기", replace: "키 교체",
    billing: "API는 구독과 별도 과금됩니다. 사용을 선택하면 다음 CLI 실행부터 적용됩니다.",
    saved: "저장됨 · 확인 전", check: "연결 확인", checked: "연결 확인됨", checking: "연결 확인 중…",
    remove: "연결 삭제", confirm: "한 번 더 눌러 삭제", noQuota: "API · 별도 과금", console: "키 발급 페이지 열기 ↗",
    checkNote: "모델 목록만 조회합니다. 생성 권한·잔액은 확인하지 않습니다.", keyNote: "키는 OS 보안 저장소에 보관하며 다시 표시하지 않습니다.",
    claudeNote: "Claude 키는 발급할 때 사용할 워크스페이스를 지정하세요.", invalid_key: "키 교체 필요", forbidden: "조회 권한 확인 필요", rate_limited: "요청 제한 · 다시 확인", unavailable: "연결 확인 필요",
    complete: "API 사용 중", use: "사용", stop: "해제", stopTitle: "API 사용 해제 · 이전 설정 복원", active: "API · 사용 중", applyFailed: "키는 저장됐지만 적용하지 못했습니다", savedOnly: "키 저장 완료", applied: "다음 실행부터 이 API를 사용합니다. 구독으로 돌아가려면 계정 목록에서 전환하세요.", nameHint: "예: work · 영문, 숫자, - 또는 _", replacement: "새 키로 교체하면 다음 CLI 실행부터 적용됩니다.",
  },
  en: {
    title: "API connections", add: "Add API key", name: "Connection name", key: "API key", save: "Save and use", cancel: "Close", replace: "Replace key",
    billing: "API usage is billed separately. Selecting Use applies this key to the next CLI session.",
    saved: "Saved · not checked", check: "Check connection", checked: "Connection checked", checking: "Checking connection…",
    remove: "Delete connection", confirm: "Click again to delete", noQuota: "API · separate billing", console: "Open API key page ↗",
    checkNote: "Checks the model list only, not generation access or balance.", keyNote: "Keys stay in the OS secure store and are never displayed again.",
    claudeNote: "Select a workspace when creating your Claude API key.", invalid_key: "Replace key", forbidden: "Check model permissions", rate_limited: "Rate limited · retry", unavailable: "Check connection",
    complete: "API active", use: "Use", stop: "Stop", stopTitle: "Stop using API · restore previous settings", active: "API · active", applyFailed: "Key saved, but could not apply it", savedOnly: "Key saved", applied: "Your next CLI session uses this API. Select a subscription account to switch back.", nameHint: "e.g. work · letters, numbers, - or _", replacement: "The replacement key applies to the next CLI launch.",
  },
};
export function apiText() { return currentLang() === "ko" ? messages.ko : messages.en; }
let apiPending = 0;
export function apiIsBusy(): boolean { return apiPending > 0 || document.querySelector(".api-dialog") !== null; }
type CheckState = "unchecked" | "ready" | "invalid_key" | "forbidden" | "rate_limited" | "unavailable";
type Connection = { provider: string; name: string; active?: boolean; checked_at?: number | null; check_state?: CheckState };
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
  head.title = currentLang() === "ko" ? "잡고 창 이동" : "Drag to move window";
  const heading = document.createElement("h2"); heading.id = "api-dialog-title"; heading.textContent = title;
  host.setAttribute("aria-labelledby", heading.id);
  const close = iconButton("close", apiText().cancel, "api-close"); close.onclick = () => dismiss();
  head.append(heading, close); host.append(head);
  // showModal makes the original titlebar inert. Keep both the modal heading
  // and the exposed titlebar strip draggable without touching the form's DOM.
  host.addEventListener("mousedown", event => {
    if (event.button !== 0 || event.detail > 1) return;
    const target = event.target;
    if (!(target instanceof Element) || target.closest("button, input, textarea, select, a")) return;
    const bar = document.querySelector(".titlebar")?.getBoundingClientRect();
    const onTitlebar = target === host && bar && event.clientX >= bar.left && event.clientX <= bar.right && event.clientY >= bar.top && event.clientY <= bar.bottom;
    if (!head.contains(target) && !onTitlebar) return;
    event.preventDefault();
    void getCurrentWindow().startDragging().catch(() => hooks.toast(currentLang() === "ko" ? "창을 이동하지 못했습니다. 다시 시도하세요." : "Could not move the window. Try again.", true));
  });
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

export function openConnectionRegistration(hooks: Hooks, provider: string, login: () => void) {
  openApiRegistration(hooks, provider, undefined, login);
}

export function openApiRegistration(hooks: Hooks, initialProvider = "claude", existing?: Connection, login?: () => void) {
  if (apiIsBusy()) return;
  const text = apiText();
  let saving = false;
  const providerTitle = initialProvider === "claude" ? "Claude" : "Codex";
  const host = dialog(hooks, existing ? text.replace : login ? `${providerTitle} · ${currentLang() === "ko" ? "계정 추가" : "Add account"}` : text.add, () => !saving);
  let provider = existing?.provider ?? initialProvider;
  const form = document.createElement("form"); form.className = "api-form";
  const providers = document.createElement("div"); providers.className = "api-providers"; providers.hidden = Boolean(login); providers.setAttribute("aria-label", "API provider");
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
  const budgetLabel = document.createElement("label"); budgetLabel.textContent = currentLang() === "ko" ? "월 예산 (USD)" : "Monthly budget (USD)";
  const budget = document.createElement("input"); budget.type = "number"; budget.name = "budget-usd"; budget.min = "0.01"; budget.max = "1000000000"; budget.step = "0.01"; budget.value = "195"; budget.required = !existing; budgetLabel.hidden = Boolean(existing); budgetLabel.append(budget);
  const errorLine = paragraph("", "api-error"); errorLine.setAttribute("role", "alert");
  const save = document.createElement("button"); save.type = "submit"; save.className = "primary api-save"; save.append(icon("shield"), text.save);
  form.append(providers, consoleLink, nameLabel, keyLabel, budgetLabel, paragraph(existing ? text.replacement : text.keyNote), providerNote, errorLine, save);
  const billing = paragraph(text.billing, "api-billing");
  let loginButton: HTMLButtonElement | undefined;
  if (login) {
    const methods = document.createElement("div"); methods.className = "api-providers connection-methods";
    const subscription = document.createElement("div"); subscription.className = "connection-subscription";
    subscription.append(paragraph(currentLang() === "ko" ? "브라우저에서 로그인해 구독 계정을 추가합니다." : "Sign in through your browser to add a subscription account."));
    loginButton = document.createElement("button"); loginButton.className = "primary connection-login"; loginButton.textContent = currentLang() === "ko" ? "로그인 시작" : "Sign in";
    loginButton.onclick = () => { if (saving) return; host.dispatchEvent(new Event("api-dismiss")); login(); };
    subscription.append(loginButton);
    for (const [method, label] of [["subscription", currentLang() === "ko" ? "구독 로그인" : "Subscription"], ["api", "API 키"]]) {
      const button = document.createElement("button"); button.type = "button"; button.dataset.method = method; button.textContent = label; button.setAttribute("aria-pressed", String(method === "subscription"));
      button.onclick = () => {
        if (saving) return;
        key.value = ""; form.hidden = billing.hidden = method !== "api"; subscription.hidden = method === "api";
        methods.querySelectorAll("button").forEach(choice => choice.setAttribute("aria-pressed", String(choice === button)));
        hooks.fit(); if (method === "api") name.focus(); else loginButton!.focus();
      };
      methods.append(button);
    }
    form.hidden = billing.hidden = true;
    host.append(methods, subscription);
  }
  host.append(billing, form);
  // A save is a short local mutation. Network checks may continue after the dialog closes.
  form.onsubmit = async event => {
    event.preventDefault(); if (saving) return;
    saving = true; apiPending++; save.disabled = true; name.readOnly = true; key.readOnly = true; choices.forEach(b => { b.disabled = true; });
    const close = host.querySelector<HTMLButtonElement>(".api-close")!; close.disabled = true;
    const connection: Connection = { provider, name: name.value.trim().toLowerCase() };
    const apply = async () => {
      if (!existing) await invoke("api_usage_save", { ...connection, settings: { budget_usd: Number(budget.value), source: "manual", spent_usd: null, api_key_id: null, admin_key: null } });
      await invoke("api_activate", connection);
    };
    let stored = false;
    let activationError: string | null = null;
    try {
      await invoke("api_save", { ...connection, key: key.value.trim(), replace: Boolean(existing) }); stored = true;
      key.value = "";
      try { await apply(); }
      catch (error) { activationError = String(error); }
    } catch (error) { errorLine.textContent = String(error); }
    finally {
      key.value = ""; saving = false; apiPending--; close.disabled = false; save.disabled = false;
      name.readOnly = Boolean(existing); key.readOnly = false; choices.forEach(b => { b.disabled = Boolean(existing); });
    }
    if (!stored) { key.focus(); return; }
    form.remove(); host.querySelector(".connection-methods")?.remove(); host.querySelector(".connection-subscription")?.remove(); host.querySelector("h2")!.textContent = activationError ? text.savedOnly : text.complete;
    const result = paragraph(activationError ? `${text.applyFailed}: ${activationError}` : text.applied, activationError ? "api-error" : "api-applied");
    host.append(result);
    if (activationError) {
      const retry = document.createElement("button"); retry.className = "primary api-apply"; retry.textContent = text.use;
      retry.onclick = async () => {
        if (saving) return;
        saving = true; apiPending++; retry.disabled = true; close.disabled = true;
        try { await apply(); result.className = "api-applied"; result.textContent = text.applied; host.querySelector("h2")!.textContent = text.complete; retry.remove(); }
        catch (error) { result.textContent = String(error); retry.disabled = false; }
        finally { saving = false; apiPending--; close.disabled = false; hooks.fit(); }
      };
      host.append(retry);
    }
    const status = paragraph(text.checking, "api-check-result"); status.setAttribute("role", "status"); host.append(status, paragraph(text.checkNote));
    hooks.fit();
    try { const checked = await invoke<Connection>("api_check", connection); if (host.isConnected) { status.textContent = stateLabel(checked); status.dataset.state = "ready"; } }
    catch (error) { if (host.isConnected) { status.textContent = String(error); status.dataset.state = "error"; } }
    finally { if (!host.isConnected) void hooks.changed(); hooks.fit(); }
  };
  host.showModal(); hooks.fit(); (loginButton ?? (existing ? key : name)).focus();
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
    card.classList.toggle("api-active", Boolean(connection.active));
    const badge = document.createElement("span"); badge.className = "api-badge"; badge.textContent = connection.active ? text.active : "API";
    const status = paragraph(stateLabel(connection), "api-status"); status.dataset.state = connection.check_state ?? "unchecked";
    status.setAttribute("role", "status");
    status.title = text.noQuota + (connection.checked_at ? " · " + new Date(connection.checked_at * 1000).toLocaleString() : "");
    head.append(name, badge, status);
    const actions = document.createElement("div"); actions.className = "api-actions";
    const use = document.createElement("button"); use.className = "api-use"; use.type = "button";
    use.textContent = connection.active ? text.stop : text.use;
    use.title = connection.active ? text.stopTitle : text.use;
    use.dataset.apiFocus = `${provider}/${connection.name}/use`;
    use.onclick = async () => {
      if (apiPending) return;
      apiPending++; use.disabled = true;
      let changed = false;
      try { await invoke(connection.active ? "api_deactivate" : "api_activate", { provider, name: connection.name }); changed = true; }
      catch (error) { status.textContent = String(error); status.dataset.state = "error"; }
      finally { apiPending--; use.disabled = false; }
      if (changed) await hooks.changed();
    };
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
      let changed = false;
      try { await invoke("api_delete", { provider, name: connection.name }); changed = true; }
      catch (error) { status.textContent = String(error); status.dataset.state = "error"; }
      finally { apiPending--; remove.disabled = false; hooks.fit(); }
      if (changed) await hooks.changed();
    };
    actions.append(use, check, replace, remove); card.append(head, actions, await apiUsageRow(provider, connection.name, hooks)); panel.append(card);
  }
  return panel;
}


type ApiUsage = {
  month: string; budget_usd: number; spent_usd: number | null; remaining_usd: number | null; percent: number | null;
  source: "manual" | "automatic"; api_key_id: string; admin_key_stored: boolean;
  checked_at: number | null; provider_updated_at: string | null; error: string | null;
};
const money = (amount: number) => new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 2 }).format(amount);
const defaultUsage = (): ApiUsage => ({ month: new Date().toISOString().slice(0, 7), budget_usd: 195, spent_usd: null, remaining_usd: null, percent: null, source: "manual", api_key_id: "", admin_key_stored: false, checked_at: null, provider_updated_at: null, error: null });

function usageMeter(usage: ApiUsage, compact = false): HTMLElement {
  const ko = currentLang() === "ko";
  const box = document.createElement("div"); box.className = `api-cost-meter${compact ? " compact-cost-meter" : ""}`;
  const top = document.createElement("div"); top.className = "api-cost-heading";
  if (!compact) { const month = document.createElement("span"); month.textContent = `${usage.month} · UTC`; top.append(month); }
  const amount = document.createElement("strong"); amount.className = "api-cost-amount";
  amount.textContent = `${usage.spent_usd === null ? "—" : money(usage.spent_usd)} / ${compact ? new Intl.NumberFormat("en-US").format(usage.budget_usd) : money(usage.budget_usd)}`;
  top.append(amount);
  const track = document.createElement("div"); track.className = "api-cost-track";
  const fill = document.createElement("span"); fill.style.width = `${Math.min(100, Math.max(0, usage.percent ?? 0))}%`; track.append(fill);
  if (usage.percent !== null) { track.setAttribute("role", "progressbar"); track.setAttribute("aria-valuenow", String(Math.min(100, Math.max(0, usage.percent)))); track.setAttribute("aria-valuemin", "0"); track.setAttribute("aria-valuemax", "100"); track.setAttribute("aria-label", ko ? "API 월 예산 사용률" : "API monthly budget usage"); }
  box.classList.toggle("cost-warning", (usage.percent ?? 0) >= 80); box.classList.toggle("cost-exceeded", (usage.percent ?? 0) >= 100);
  box.append(top, track);
  const detail = usage.spent_usd === null ? (ko ? "사용 금액 미입력 · 조회 전" : "Amount not entered or fetched") : usage.remaining_usd! < 0 ? (ko ? `${money(-usage.remaining_usd!)} 초과` : `${money(-usage.remaining_usd!)} over budget`) : (ko ? `${money(usage.remaining_usd!)} 남음` : `${money(usage.remaining_usd!)} remaining`);
  box.title = `${detail} · ${usage.source === "manual" ? (ko ? "직접 입력" : "Manual") : (ko ? "API 집계 · 반영 지연 가능" : "API report · may be delayed")}${usage.checked_at ? " · " + new Date(usage.checked_at * 1000).toLocaleString() : ""}${usage.provider_updated_at ? " · " + usage.provider_updated_at : ""}${usage.error ? " · " + usage.error : ""}`;
  if (!compact) {
    box.append(paragraph(`${detail} · ${usage.source === "manual" ? (ko ? "직접 입력" : "Manual") : (ko ? "자동 조회" : "Automatic")}`, "api-cost-detail"));
    if (usage.error) box.append(paragraph(usage.error, "api-cost-error"));
  }
  return box;
}

async function apiUsageRow(provider: string, name: string, hooks: Hooks): Promise<HTMLElement> {
  let usage: ApiUsage;
  try { usage = await invoke<ApiUsage>("api_usage_read", { provider, name }); }
  catch (error) { usage = { ...defaultUsage(), error: String(error) }; }
  const row = document.createElement("div"); row.className = "api-cost";
  const meter = document.createElement("div"); meter.className = "api-cost-content"; meter.append(usageMeter(usage));
  const actions = document.createElement("div"); actions.className = "api-cost-actions";
  const edit = iconButton("edit", currentLang() === "ko" ? "사용량·예산 설정" : "Usage and budget settings", "api-usage-settings");
  edit.dataset.apiFocus = `${provider}/${name}/usage`;
  edit.onclick = () => openUsageSettings(provider, name, usage, hooks);
  const refresh = iconButton("reconnect", currentLang() === "ko" ? "비용 새로고침" : "Refresh costs", "api-usage-refresh");
  const repaint = () => { meter.replaceChildren(usageMeter(usage)); refresh.hidden = usage.source !== "automatic"; hooks.fit(); };
  const update = async (force: boolean) => {
    refresh.disabled = true;
    try { usage = await invoke<ApiUsage>("api_usage_refresh", { provider, name, force }); }
    catch (error) { usage = { ...usage, error: String(error) }; }
    finally { refresh.disabled = false; repaint(); }
  };
  refresh.hidden = usage.source !== "automatic"; refresh.onclick = () => void update(true);
  actions.append(edit, refresh); row.append(meter, actions);
  if (usage.source === "automatic") void update(false);
  return row;
}

export function compactApiUsage(provider: string, name: string): HTMLElement {
  const row = document.createElement("div"); row.className = "compact-api-active";
  const label = document.createElement("div"); label.className = "compact-api-label";
  const badge = document.createElement("strong"); badge.textContent = "API";
  const title = document.createElement("span"); title.textContent = name; label.append(badge, title);
  row.title = `API · ${name}`; const meter = document.createElement("div"); meter.className = "compact-api-cost";
  meter.append(usageMeter(defaultUsage(), true)); row.append(label, meter);
  void invoke<ApiUsage>("api_usage_read", { provider, name }).then(async usage => {
    meter.replaceChildren(usageMeter(usage, true));
    if (usage.source === "automatic") { const updated = await invoke<ApiUsage>("api_usage_refresh", { provider, name, force: false }); meter.replaceChildren(usageMeter(updated, true)); }
  }).catch(error => { meter.title = String(error); meter.classList.add("cost-warning"); });
  return row;
}

function openUsageSettings(provider: string, name: string, current: ApiUsage, hooks: Hooks) {
  if (apiIsBusy()) return;
  const ko = currentLang() === "ko"; let saving = false;
  const host = dialog(hooks, ko ? `${name} · 사용량` : `${name} · Usage`, () => !saving);
  const form = document.createElement("form"); form.className = "api-form api-usage-form";
  const input = (label: string, field: string, type: string, value: string) => {
    const wrapper = document.createElement("label"); wrapper.textContent = label;
    const control = document.createElement("input"); control.name = field; control.type = type; control.value = value; wrapper.append(control); return { wrapper, control };
  };
  const budget = input(ko ? "월 예산 (USD)" : "Monthly budget (USD)", "budget", "number", String(current.budget_usd));
  budget.control.required = true; budget.control.min = "0.01"; budget.control.max = "1000000000"; budget.control.step = "0.01";
  const methodLabel = document.createElement("label"); methodLabel.textContent = ko ? "사용 금액 입력 방법" : "Cost source";
  const method = document.createElement("select"); method.name = "source";
  for (const [value, label] of [["manual", ko ? "직접 입력" : "Manual"], ["automatic", ko ? "API에서 자동 조회" : "Automatic API reporting"]]) { const option = document.createElement("option"); option.value = value; option.textContent = label; method.append(option); }
  method.value = current.source; methodLabel.append(method);
  const manual = input(ko ? `${current.month} 사용 금액 (USD)` : `${current.month} spent (USD)`, "spent", "number", current.source === "manual" && current.spent_usd !== null ? String(current.spent_usd) : "");
  manual.control.min = "0"; manual.control.max = "1000000000"; manual.control.step = "0.01"; manual.control.placeholder = ko ? "미입력 상태 유지 가능" : "Leave blank if unknown";
  const automatic = document.createElement("div"); automatic.className = "api-usage-auto";
  const keyId = input("API key ID", "key-id", "text", current.api_key_id); keyId.control.placeholder = provider === "claude" ? "apikey_…" : "key_…"; keyId.control.maxLength = 200;
  const admin = input(ko ? "비용 조회용 관리자 API 키" : "Admin API key for cost reports", "admin-key", "password", ""); admin.control.autocomplete = "new-password"; admin.control.maxLength = 4096; admin.control.minLength = 16; admin.control.placeholder = current.admin_key_stored ? (ko ? "저장됨 · 비워 두면 기존 키 유지" : "Saved · leave blank to keep") : "sk-…";
  automatic.append(keyId.wrapper, admin.wrapper, paragraph(ko ? "관리자 권한이 있는 조회용 키가 필요합니다. API key ID는 조회할 실행 키의 식별자이며 비밀 키와 다릅니다." : "Requires an administrative reporting key. API key ID identifies the inference key to report, not its secret."));
  const console = document.createElement("button"); console.type = "button"; console.className = "api-console"; console.textContent = ko ? "공급자 비용 페이지 ↗" : "Provider cost dashboard ↗";
  const error = paragraph("", "api-error"); error.setAttribute("role", "alert");
  console.onclick = () => { void invoke("api_open_usage_console", { provider }).catch(reason => { error.textContent = String(reason); }); };
  const save = document.createElement("button"); save.type = "submit"; save.className = "primary api-save";
  const change = () => {
    const auto = method.value === "automatic"; manual.wrapper.hidden = auto; automatic.hidden = !auto; keyId.control.required = auto; admin.control.required = auto && !current.admin_key_stored; admin.control.value = "";
    save.textContent = auto ? (ko ? "저장하고 조회" : "Save and fetch") : (ko ? "저장" : "Save"); hooks.fit();
  };
  method.onchange = change;
  form.append(budget.wrapper, methodLabel, manual.wrapper, automatic, console, paragraph(ko ? "API별 월 예산이며 UTC 기준으로 월이 바뀝니다. 공급자의 결제 한도나 실행 중인 CLI를 차단하지 않습니다." : "A monthly budget for this API, using UTC months. This does not set a provider billing limit or stop a running CLI."), error, save);
  host.append(form); change();
  form.onsubmit = async event => {
    event.preventDefault(); if (saving) return;
    saving = true; apiPending++; save.disabled = true; host.querySelector<HTMLButtonElement>(".api-close")!.disabled = true;
    let saved = false;
    try {
      await invoke("api_usage_save", { provider, name, settings: { budget_usd: Number(budget.control.value), source: method.value, spent_usd: method.value === "manual" && manual.control.value !== "" ? Number(manual.control.value) : null, api_key_id: keyId.control.value.trim(), admin_key: admin.control.value.trim() || null } }); saved = true;
    } catch (reason) { error.textContent = String(reason); }
    finally { admin.control.value = ""; saving = false; apiPending--; save.disabled = false; host.querySelector<HTMLButtonElement>(".api-close")!.disabled = false; }
    if (saved) host.dispatchEvent(new Event("api-dismiss"));
  };
  host.showModal(); hooks.fit(); budget.control.focus();
}
