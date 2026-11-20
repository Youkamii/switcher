import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { currentLang, t } from "./i18n";
import { icon, iconButton, setButtonIcon } from "./uiIcons";
import { readConsoleCredit } from "./consoleCredit";

const messages = {
  ko: {
    title: "API 연결", add: "API 키 등록", name: "연결 이름", key: "API 키", save: "저장하고 사용", cancel: "닫기", replace: "키 교체",
    billing: "API는 구독과 별도 과금됩니다. 사용을 선택하면 다음 CLI 실행부터 적용됩니다.",
    saved: "저장됨 · 확인 전", check: "연결 확인", checked: "연결 확인됨", checking: "연결 확인 중…",
    remove: "연결 삭제", confirm: "한 번 더 눌러 삭제", noQuota: "API · 별도 과금", console: "키 발급 페이지 열기 ↗",
    checkNote: "모델 목록으로 연결을 확인합니다. 선불 잔액은 API 카드에서 확인하세요.", keyNote: "키는 OS 보안 저장소에 보관하며 다시 표시하지 않습니다.",
    claudeNote: "Claude 키는 발급할 때 사용할 워크스페이스를 지정하세요.", invalid_key: "키 교체 필요", forbidden: "조회 권한 확인 필요", rate_limited: "요청 제한 · 다시 확인", unavailable: "연결 확인 필요",
    complete: "API 사용 중", use: "사용", stop: "해제", stopTitle: "API 사용 해제 · 이전 설정 복원", active: "API · 사용 중", applyFailed: "키는 저장됐지만 적용하지 못했습니다", savedOnly: "키 저장 완료", applied: "다음 실행부터 이 API를 사용합니다. 구독으로 돌아가려면 계정 목록에서 전환하세요.", nameHint: "예: work · 영문, 숫자, - 또는 _", replacement: "새 키로 교체하면 다음 CLI 실행부터 적용됩니다.",
  },
  en: {
    title: "API connections", add: "Add API key", name: "Connection name", key: "API key", save: "Save and use", cancel: "Close", replace: "Replace key",
    billing: "API usage is billed separately. Selecting Use applies this key to the next CLI session.",
    saved: "Saved · not checked", check: "Check connection", checked: "Connection checked", checking: "Checking connection…",
    remove: "Delete connection", confirm: "Click again to delete", noQuota: "API · separate billing", console: "Open API key page ↗",
    checkNote: "The model list checks the connection. Open the API card for prepaid balance information.", keyNote: "Keys stay in the OS secure store and are never displayed again.",
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
  const host = dialog(hooks, existing ? text.replace : login ? `${providerTitle} · ${t("addConnection").replace(/^[+＋]\s*/, "")}` : text.add, () => !saving);
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
  const errorLine = paragraph("", "api-error"); errorLine.setAttribute("role", "alert");
  const save = document.createElement("button"); save.type = "submit"; save.className = "primary api-save"; save.append(icon("shield"), text.save);
  form.append(providers, consoleLink, nameLabel, keyLabel, paragraph(existing ? text.replacement : text.keyNote), providerNote, errorLine, save);
  const billing = paragraph(text.billing, "api-billing");
  let loginButton: HTMLButtonElement | undefined;
  if (login) {
    const methods = document.createElement("div"); methods.className = "api-providers connection-methods";
    const subscription = document.createElement("div"); subscription.className = "connection-subscription";
    subscription.append(paragraph(currentLang() === "ko" ? "브라우저에서 로그인해 구독 계정을 추가합니다." : "Sign in through your browser to add a subscription account."));
    loginButton = document.createElement("button"); loginButton.className = "primary connection-login"; loginButton.textContent = currentLang() === "ko" ? "로그인 시작" : "Sign in";
    loginButton.onclick = () => { if (saving) return; host.dispatchEvent(new Event("api-dismiss")); login(); };
    subscription.append(loginButton);
    for (const [method, label] of [["subscription", currentLang() === "ko" ? "구독 로그인" : "Subscription"], ["api", text.add]]) {
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
    const apply = () => invoke("api_activate", connection);
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

export async function apiConnections(provider: string, hooks: Hooks, force: boolean): Promise<HTMLElement> {
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
    const balance = balanceDisplay(provider, connection.name, false, hooks);
    void balance.refresh(force);
    const check = iconButton("reconnect", provider === "claude" ? balanceText().refresh : text.check, "api-check");
    check.onclick = async () => {
      check.disabled = true; status.textContent = text.checking;
      void balance.refresh(true);
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
    actions.append(use, check, replace, remove); card.append(head, actions, balance.element); panel.append(card);
  }
  return panel;
}

type BalanceIssue = "unsupported" | "login_required" | "login_expired" | "key_rejected" | "forbidden" | "rate_limited" | "unavailable";
type Credit = { amount_minor: number; currency: string; checked_at: number };
type Balance = { credit: Credit | null; issue: BalanceIssue | null; organization: string | null; console_credit: Credit | null; import_ref: string | null };

function balanceText() {
  return currentLang() === "ko" ? {
    label: "조직 선불 잔액", checking: "조회 중…", time: "확인 시각", billing: "결제 화면 ↗", refresh: "연결 및 잔액 새로고침",
    unsupported: "결제 화면에서 확인", login_required: "키를 발급한 Claude Console에서 확인", login_expired: "Claude 계정 다시 연결 필요",
    key_rejected: "API 키 확인 필요", forbidden: "잔액 조회 권한 없음", rate_limited: "잠시 후 다시 조회", unavailable: "잔액 조회 실패",
    consoleLabel: "조직 크레딧", copied: "Console 확인", paste: "대시보드 가져오기", pasteHint: "Claude Console 대시보드에서 조직 이름과 ‘조직 크레딧’ 금액을 함께 복사한 뒤 여기에 붙여넣으세요.",
    pasteError: "이 연결의 조직 이름과 ‘조직 크레딧’ 금액을 함께 복사하세요.", pasteHere: "대시보드 내용 붙여넣기",
  } : {
    label: "Organization prepaid balance", checking: "Checking…", time: "Checked", billing: "View billing ↗", refresh: "Refresh connection and balance",
    unsupported: "Check billing", login_required: "Check the Claude Console that issued this key", login_expired: "Reconnect your Claude account",
    key_rejected: "Check API key", forbidden: "Balance access denied", rate_limited: "Try again shortly", unavailable: "Balance unavailable",
    consoleLabel: "Organization credits", copied: "From Console", paste: "Import dashboard", pasteHint: "Copy the organization name and Organization credits amount from your Claude Console dashboard, then paste them here.",
    pasteError: "Copy this connection’s organization name together with its Organization credits amount.", pasteHere: "Paste dashboard text",
  };
}

function openConsoleImport(hooks: Hooks, name: string, organization: string, reference: string) {
  const text = balanceText();
  let saving = false;
  const host = dialog(hooks, text.paste, () => !saving);
  const identity = paragraph(organization, "api-balance-org");
  const dashboard = document.createElement("button"); dashboard.type = "button"; dashboard.className = "api-billing-link";
  dashboard.textContent = currentLang() === "ko" ? "대시보드 열기 ↗" : "Open dashboard ↗";
  dashboard.onclick = () => { void invoke("api_open_billing", { provider: "claude", dashboard: true }).catch(cause => hooks.toast(String(cause), true)); };
  const entry = document.createElement("textarea");
  entry.className = "api-credit-paste"; entry.rows = 2; entry.spellcheck = false;
  entry.placeholder = text.pasteHere; entry.setAttribute("aria-label", text.pasteHere);
  entry.autocomplete = "off";
  const error = paragraph("", "api-error"); error.setAttribute("role", "status");
  async function save(raw: string) {
    if (saving) return;
    const amount = readConsoleCredit(raw, organization);
    if (amount === undefined) { error.textContent = text.pasteError; return; }
    saving = true; entry.disabled = true;
    try {
      await invoke("api_import_console_credit", { name, reading: { organization, import_ref: reference, amount_minor: amount } });
      saving = false; host.dispatchEvent(new Event("api-dismiss"));
    } catch (cause) { saving = false; entry.disabled = false; error.textContent = String(cause); entry.focus(); }
  }
  entry.onpaste = event => {
    if (!event.clipboardData) return;
    event.preventDefault(); void save(event.clipboardData.getData("text/plain"));
  };
  entry.oninput = () => { const raw = entry.value; entry.value = ""; void save(raw); };
  host.append(identity, dashboard, paragraph(text.pasteHint), entry, error);
  host.showModal(); hooks.fit(); entry.focus();
}

function formatCredit(credit: Credit): string | undefined {
  if (!Number.isSafeInteger(credit.amount_minor)) return;
  try {
    // The provider reports minor units. Intl knows USD has two decimals, JPY zero, KWD three.
    const intl = Intl as typeof Intl & { supportedValuesOf?: (kind: "currency") => string[] };
    if (intl.supportedValuesOf && !intl.supportedValuesOf("currency").includes(credit.currency)) return;
    const format = new Intl.NumberFormat(currentLang(), { style: "currency", currency: credit.currency });
    const digits = format.resolvedOptions().maximumFractionDigits;
    if (digits === undefined) return;
    return format.format(credit.amount_minor / 10 ** digits);
  } catch { return; }
}

function balanceDisplay(provider: string, name: string, compact: boolean, hooks: Pick<Hooks, "fit" | "toast"> & Partial<Pick<Hooks, "changed">>) {
  const text = balanceText();
  const element = document.createElement("div");
  element.className = compact ? "api-balance compact-balance" : "api-balance";
  const label = document.createElement("span");
  label.className = "api-balance-label";
  const caption = document.createTextNode(text.label); label.append(caption);
  const organization = document.createElement("span");
  organization.className = "api-balance-org";
  label.append(organization);
  const value = document.createElement("strong"); value.className = "api-balance-value";
  value.setAttribute("role", "status");
  const openBilling = () => { void invoke("api_open_billing", { provider }).catch(error => hooks.toast?.(String(error), true)); };
  const detail = document.createElement(compact ? "button" : "span"); detail.className = "api-balance-detail";
  if (detail instanceof HTMLButtonElement) {
    detail.type = "button"; detail.onclick = openBilling; detail.title = text.billing;
    detail.classList.add("api-billing-link");
  }
  element.append(label, value, detail);
  let importer: HTMLButtonElement | undefined;
  if (!compact) {
    const links = document.createElement("div"); links.className = "api-balance-links";
    if (provider === "claude" && hooks.changed) {
      importer = document.createElement("button"); importer.type = "button"; importer.className = "api-credit-import";
      importer.textContent = text.paste; importer.hidden = true; links.append(importer);
    }
    const link = document.createElement("button"); link.type = "button"; link.className = "api-billing-link";
    link.textContent = text.billing;
    link.onclick = openBilling;
    links.append(link); element.append(links);
  }
  function paint(data: Partial<Balance>) {
    const { issue, organization: org, import_ref: reference } = data;
    const saved = !data.credit && data.console_credit;
    const credit = data.credit || saved;
    caption.textContent = saved ? text.consoleLabel : text.label;
    organization.textContent = org ? " · " + org : "";
    if (importer) {
      importer.hidden = !(org && reference && !data.credit);
      importer.onclick = org && reference && hooks.changed ? () => openConsoleImport({ ...hooks, changed: hooks.changed! }, name, org, reference) : null;
    }
    const amount = credit && formatCredit(credit);
    const status = issue ? text[issue] : credit && !amount ? text.unavailable : "";
    const time = credit && amount ? new Date(credit.checked_at * 1000) : undefined;
    const when = time ? (saved ? text.copied : text.time) + " " + time.toLocaleString(currentLang()) : "";
    const shortTime = time?.toLocaleString(currentLang(), { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
    value.textContent = amount || (status ? "—" : text.checking);
    detail.textContent = saved && shortTime ? text.copied + " · " + shortTime : compact && status ? status + " ↗" : status;
    detail.hidden = !detail.textContent;
    element.dataset.state = amount ? (saved ? "console" : "ready") : issue ?? (credit ? "unavailable" : "checking");
    element.title = [caption.textContent, amount, when, status].filter(Boolean).join(" · ");
    element.setAttribute("aria-label", element.title);
    if (element.isConnected) hooks.fit();
  }
  let generation = 0;
  paint({ issue: provider === "codex" ? "unsupported" : null });
  async function refresh(force: boolean) {
    if (provider !== "claude") return;
    const version = ++generation;
    try {
      const data = await invoke<Balance>("api_balance", { provider, name, force });
      if (generation === version) paint(data);
    } catch {
      if (generation === version) paint({issue: "unavailable"});
    }
  }
  return { element, refresh };
}

export function compactApiBalance(provider: string, name: string, force: boolean, hooks: Pick<Hooks, "fit" | "toast">): HTMLElement {
  const row = document.createElement("div"); row.className = "compact-api-active";
  const label = document.createElement("div"); label.className = "compact-api-label";
  const badge = document.createElement("strong"); badge.textContent = "API";
  const title = document.createElement("span"); title.textContent = name; label.append(badge, title);
  row.title = `API · ${name}`;
  const balance = balanceDisplay(provider, name, true, hooks);
  row.append(label, balance.element);
  void balance.refresh(force);
  return row;
}
