import { invoke } from "@tauri-apps/api/core";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { currentLang, setLang, t } from "./i18n";

type ProfileInfo = {
  name: string;
  id: string;
  email: string | null;
  plan: string | null;
  plan_tier: number | null;
  saved_at: number;
  active: boolean;
};

type Snapshot = {
  profiles: ProfileInfo[];
  live: { id: string; email: string | null } | null;
  live_saved: boolean;
};

type UsageWindow = {
  key: string;
  label: string;
  percent: number;
  resets_at: string | null;
};

type Usage = { windows: UsageWindow[]; stale?: boolean; stale_age_secs?: number | null };

const PROVIDERS = [
  { id: "claude", title: "CLAUDE" },
  { id: "codex", title: "CODEX" },
] as const;

type ProviderId = (typeof PROVIDERS)[number]["id"];

const app = document.getElementById("app")!;
const titlebarEl = document.querySelector(".titlebar") as HTMLElement;
const toastEl = document.getElementById("toast")!;
let toastTimer: number | undefined;
let rendering = false;
/// 로그인 패널이 열려 있으면 자동 새로고침이 화면을 갈아엎지 않게 한다
let loginOpen = false;

function toast(message: string, isError = false) {
  toastEl.textContent = message;
  toastEl.classList.toggle("error", isError);
  toastEl.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (toastEl.hidden = true), 3500);
}

/// 리셋까지 남은 시간을 짧게 (예: "3h 42m", "5d 23h")
function formatReset(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const ts = /^\d+$/.test(resetsAt) ? Number(resetsAt) * 1000 : Date.parse(resetsAt);
  if (Number.isNaN(ts)) return "";
  const diff = ts - Date.now();
  if (diff <= 0) return "reset";
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (days >= 1) return `${days}d ${hours % 24}h`;
  if (hours >= 1) return `${hours}h ${minutes % 60}m`;
  return `${minutes}m`;
}

/// 지금 조회가 막혀 이전 수치를 보여줄 때의 라벨 — 몇 시간 전 값인지 감추지 않는다.
/// formatReset과 같은 내림(floor) 방식 (반올림이면 3599초가 "60분 전 값"이 된다)
function staleLabel(usage: Usage): string {
  const secs = usage.stale_age_secs;
  if (secs == null) return t("usageWaiting");
  if (secs < 3600) return t("staleMin", { n: Math.max(1, Math.floor(secs / 60)) });
  return t("staleHour", { n: Math.floor(secs / 3600) });
}

/// 컴팩트용 나이 축약 ("3h 전", "45m 전") — 컴팩트의 축약 표기 관례를 따른다
function compactStaleAge(secs: number | null | undefined): string {
  if (secs == null) return "";
  if (secs < 3600) return t("agoMin", { n: Math.max(1, Math.floor(secs / 60)) });
  return t("agoHour", { n: Math.floor(secs / 3600) });
}

function usageRow(win: UsageWindow): HTMLElement {
  const row = document.createElement("div");
  row.className = "usage-row";

  const label = document.createElement("span");
  label.className = "usage-label";
  label.textContent = win.label;
  label.title = win.label;

  const bar = document.createElement("div");
  bar.className = "bar";
  const fill = document.createElement("div");
  fill.className = "bar-fill";
  if (win.percent >= 85) fill.classList.add("danger");
  else if (win.percent >= 60) fill.classList.add("warn");
  fill.style.width = `${Math.min(100, Math.max(0, win.percent))}%`;
  bar.appendChild(fill);

  const pct = document.createElement("span");
  pct.className = "usage-pct";
  pct.textContent = `${Math.round(win.percent)}%`;

  // 리셋까지 남은 시간은 모든 창에 각각 표시한다
  const reset = document.createElement("span");
  reset.className = "usage-reset";
  reset.textContent = formatReset(win.resets_at);
  reset.title = t("resetTooltip");

  row.append(label, bar, pct, reset);
  return row;
}

async function loadUsage(provider: ProviderId, card: HTMLElement, profile: string | null) {
  const box = document.createElement("div");
  box.className = "usage-box";
  const loading = document.createElement("div");
  loading.className = "usage-note";
  loading.textContent = t("loadingUsage");
  box.appendChild(loading);
  card.appendChild(box);

  try {
    const usage = await invoke<Usage>("fetch_usage", { provider, profile });
    box.textContent = "";
    if (usage.windows.length === 0) {
      const empty = document.createElement("div");
      empty.className = "usage-note";
      empty.textContent = t("noUsage");
      box.appendChild(empty);
      return;
    }
    for (const win of usage.windows) box.appendChild(usageRow(win));
    if (usage.stale) {
      // 갱신이 막힌 상태(요청 제한·토큰 만료) — 기존 수치를 살짝 흐리게 두고
      // 그 수치가 몇 분/시간 전 것인지 작게 알린다
      box.classList.add("stale");
      const overlay = document.createElement("div");
      overlay.className = "stale-overlay";
      overlay.textContent = staleLabel(usage);
      box.appendChild(overlay);
    }
  } catch (error) {
    box.textContent = "";
    const message = String(error);
    const note = document.createElement("div");
    // 보여줄 이전 수치조차 없는 초기 상태의 일시 장애는 작은 안내로만
    note.className = message.includes("조회 대기중") ? "usage-note" : "usage-error";
    note.textContent = message;
    box.appendChild(note);
  }
  fitHeight();
}

function profileCard(
  provider: ProviderId,
  profile: ProfileInfo,
  pending: Promise<unknown>[],
): HTMLElement {
  const card = document.createElement("div");
  card.className = "card" + (profile.active ? " active" : "");

  const head = document.createElement("div");
  head.className = "card-head";
  // 사용자는 이메일로 계정을 구분한다 — 프로필 이름은 안 보여주고 이메일만
  const email = document.createElement("span");
  email.className = "card-name";
  email.textContent = profile.email ?? profile.name;
  email.title = t("profileNameTooltip", { name: profile.name });
  head.append(email);
  if (profile.plan) {
    const plan = document.createElement("span");
    plan.className = "badge plan";
    plan.textContent = profile.plan;
    if (profile.plan_tier) {
      // Max 배수: 5는 노랑, 20은 빨강
      const tier = document.createElement("span");
      tier.className = `tier tier-${profile.plan_tier}`;
      tier.textContent = String(profile.plan_tier);
      plan.appendChild(tier);
    }
    head.appendChild(plan);
  }
  // 활성 표시는 배지 대신 글자색으로 — 활성만 연둣빛 흰색, 나머지는 회색 (.card.active CSS)
  card.appendChild(head);

  // 활성 프로필은 활성 파일(항상 최신 토큰), 비활성은 보관함 토큰으로 조회.
  // 프라미스는 렌더러가 모은다 — 새로고침 때 다 받아진 뒤 한 번에 교체하기 위해
  pending.push(loadUsage(provider, card, profile.active ? null : profile.name));

  let switching = false;
  const doSwitch = async (disable?: HTMLButtonElement) => {
    if (switching) return;
    switching = true;
    if (disable) disable.disabled = true;
    try {
      await invoke("switch_profile", { provider, name: profile.name });
      // 성공 안내는 따로 없다 — 활성 표시가 옮겨가는 것으로 충분하다
      await render({ immediate: true });
    } catch (error) {
      toast(String(error), true);
      if (disable) disable.disabled = false;
      switching = false;
    }
  };

  const actions = document.createElement("div");
  actions.className = "card-actions";
  if (!profile.active) {
    // 고정(위젯) 모드의 더블클릭 전환은 Rust가 시스템 차원에서 감지한다 —
    // 카드에 어느 계정인지만 새겨 둔다 (reportHitRegions가 좌표와 함께 보고)
    card.classList.add("switchable");
    card.dataset.provider = provider;
    card.dataset.name = profile.name;

    const switchBtn = document.createElement("button");
    switchBtn.className = "primary";
    switchBtn.textContent = t("switchBtn");
    switchBtn.addEventListener("click", () => void doSwitch(switchBtn));
    actions.appendChild(switchBtn);
  }

  const deleteBtn = document.createElement("button");
  deleteBtn.textContent = t("del");
  let armed = false;
  deleteBtn.addEventListener("click", async () => {
    if (!armed) {
      armed = true;
      deleteBtn.textContent = t("delConfirm");
      deleteBtn.classList.add("danger-armed");
      window.setTimeout(() => {
        armed = false;
        deleteBtn.textContent = t("del");
        deleteBtn.classList.remove("danger-armed");
      }, 3000);
      return;
    }
    deleteBtn.disabled = true;
    try {
      await invoke("delete_profile", { provider, name: profile.name });
      toast(t("delDone", { name: profile.name }));
      await render({ immediate: true });
    } catch (error) {
      toast(String(error), true);
      deleteBtn.disabled = false;
    }
  });
  actions.appendChild(deleteBtn);
  card.appendChild(actions);

  return card;
}

type LoginOutcome = {
  profile: string;
  email: string | null;
  updated_existing: boolean;
};

type LoginPrompt = {
  url: string;
  device_code: string | null;
  needs_code: boolean;
};

async function copyText(value: string, button: HTMLButtonElement, label: string) {
  try {
    await navigator.clipboard.writeText(value);
    button.textContent = t("copied");
    window.setTimeout(() => (button.textContent = label), 1500);
  } catch {
    toast(t("copyFailed"), true);
  }
}

/// 복사할 값 하나를 보여주는 상자 (로그인 주소, 일회용 코드)
function copyBox(title: string, value: string, mono: boolean): HTMLElement {
  const box = document.createElement("div");
  box.className = "copy-box";

  const head = document.createElement("div");
  head.className = "copy-title";
  head.textContent = title;

  const body = document.createElement("div");
  body.className = "copy-body";
  const text = document.createElement("div");
  text.className = mono ? "copy-value mono" : "copy-value";
  text.textContent = value;
  text.title = value;
  const btn = document.createElement("button");
  btn.textContent = t("copy");
  btn.addEventListener("click", () => void copyText(value, btn, t("copy")));
  body.append(text, btn);

  box.append(head, body);
  return box;
}

/// 로그인 패널. 성공·실패·취소 어느 경로든 onExit 하나로 끝난다 —
/// onExit은 loginOpen을 내리고 전체를 다시 그린다 (수동 복원 분기 제거).
function loginPanel(prompt: LoginPrompt, onExit: () => void): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "login-panel";

  const steps = document.createElement("div");
  steps.className = "help";
  steps.textContent = prompt.needs_code ? t("stepsClaude") : t("stepsCodex");
  panel.appendChild(steps);

  panel.appendChild(copyBox(t("loginUrl"), prompt.url, false));
  if (prompt.device_code) {
    panel.appendChild(copyBox(t("oneTimeCode"), prompt.device_code, true));
  }

  if (prompt.needs_code) {
    const actions = document.createElement("div");
    actions.className = "add-row";
    const input = document.createElement("input");
    input.placeholder = t("pasteCode");
    input.maxLength = 64;
    const okBtn = document.createElement("button");
    okBtn.className = "primary";
    okBtn.textContent = t("ok");
    const submit = async () => {
      const code = input.value.trim();
      if (!code) {
        toast(t("codeEmpty"), true);
        return;
      }
      okBtn.disabled = true;
      input.disabled = true;
      okBtn.textContent = t("okWorking");
      try {
        const result = await invoke<LoginOutcome>("submit_login_code", { code });
        reportLogin(result);
      } catch (error) {
        // 코드 제출 후의 실패는 세션이 이미 끝난 상태라 재시도가 불가능하다 —
        // 패널을 닫고 처음부터 다시 시작하게 안내한다
        toast(t("retryFromStart", { error: String(error) }), true);
      }
      onExit();
    };
    okBtn.addEventListener("click", submit);
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") void submit();
    });
    actions.append(input, okBtn);
    panel.appendChild(actions);
    window.setTimeout(() => input.focus(), 50);
  } else {
    const waiting = document.createElement("div");
    waiting.className = "usage-note";
    waiting.textContent = t("waitingBrowser");
    panel.appendChild(waiting);
    // 코덱스 장치 코드 인증은 계정에서 기본으로 꺼져 있다 — 거부당하면 여기부터 확인
    const prereq = document.createElement("div");
    prereq.className = "help";
    prereq.textContent = t("codexPrereq");
    panel.appendChild(prereq);
    void (async () => {
      try {
        const result = await invoke<LoginOutcome>("await_device_login");
        reportLogin(result);
      } catch (error) {
        toast(String(error), true);
      }
      onExit();
    })();
  }

  const cancelBtn = document.createElement("button");
  cancelBtn.className = "link";
  cancelBtn.textContent = t("cancel");
  cancelBtn.addEventListener("click", () => {
    void invoke("cancel_login");
    onExit();
  });
  panel.appendChild(cancelBtn);

  return panel;
}

function reportLogin(result: LoginOutcome) {
  const who = result.email ?? result.profile;
  toast(
    result.updated_existing
      ? t("loginUpdated", { profile: result.profile, who })
      : t("loginAdded", { profile: result.profile, who }),
  );
}

function addAccountButton(provider: ProviderId, section: HTMLElement) {
  const row = document.createElement("div");
  row.className = "add-row";

  const addBtn = document.createElement("button");
  addBtn.className = "primary";
  addBtn.textContent = t("addAccount");
  row.appendChild(addBtn);
  section.appendChild(row);

  const slot = document.createElement("div");
  section.appendChild(slot);

  addBtn.addEventListener("click", async () => {
    if (loginOpen) {
      toast(t("loginBusy"), true);
      return;
    }
    addBtn.disabled = true;
    addBtn.textContent = t("gettingLoginUrl");
    try {
      const prompt = await invoke<LoginPrompt>("start_login", { provider });
      addBtn.hidden = true;
      loginOpen = true;
      slot.appendChild(
        loginPanel(prompt, () => {
          loginOpen = false;
          void render({ immediate: true });
        }),
      );
    } catch (error) {
      toast(String(error), true);
      addBtn.disabled = false;
      addBtn.textContent = t("addAccount");
    }
  });
}

function saveForm(provider: ProviderId, section: HTMLElement) {
  const row = document.createElement("div");
  row.className = "save-row";
  const input = document.createElement("input");
  input.placeholder = t("namePlaceholder");
  input.maxLength = 32;
  const saveBtn = document.createElement("button");
  saveBtn.textContent = t("saveCurrent");
  const submit = async () => {
    const name = input.value.trim();
    if (!name) {
      toast(t("nameEmpty"), true);
      return;
    }
    try {
      await invoke("save_profile", { provider, name });
      toast(t("saveDone", { name }));
      await render({ immediate: true });
    } catch (error) {
      toast(String(error), true);
    }
  };
  saveBtn.addEventListener("click", submit);
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") void submit();
  });
  row.append(input, saveBtn);
  section.appendChild(row);
}

async function renderProvider(
  provider: ProviderId,
  title: string,
  target: DocumentFragment,
  pending: Promise<unknown>[],
) {
  const section = document.createElement("section");
  const heading = document.createElement("h2");
  heading.className = "section-title";
  heading.textContent = title;
  section.appendChild(heading);

  try {
    const snap = await invoke<Snapshot>("list_profiles", { provider });

    if (!snap.live && snap.profiles.length === 0) {
      const hint = document.createElement("p");
      hint.className = "hint";
      hint.textContent = t("noAccounts");
      section.appendChild(hint);
      addAccountButton(provider, section);
      target.appendChild(section);
      return;
    }

    if (snap.live && !snap.live_saved) {
      const hint = document.createElement("p");
      hint.className = "hint warn";
      hint.textContent = t("liveNotSaved", { account: snap.live.email ?? snap.live.id });
      section.appendChild(hint);
    }

    for (const profile of snap.profiles) {
      section.appendChild(profileCard(provider, profile, pending));
    }

    addAccountButton(provider, section);
    if (snap.live && !snap.live_saved) {
      // 지금 로그인된 계정이 아직 프로필로 없을 때만 수동 저장 입력칸을 보여준다
      saveForm(provider, section);
    }
  } catch (error) {
    const err = document.createElement("p");
    err.className = "usage-error";
    err.textContent = String(error);
    section.appendChild(err);
  }

  target.appendChild(section);
}

type GithubAccount = { login: string; active: boolean };
type GithubSnapshot = { gh_found: boolean; accounts: GithubAccount[] };

/// GITHUB 계정 카드 — 사용량 없음: 이름·활성 표시·전환뿐.
/// 토큰은 위젯이 만지지 않는다 (gh가 keyring에 관리, 전환은 gh auth switch 대행)
function githubCard(acc: GithubAccount): HTMLElement {
  const card = document.createElement("div");
  card.className = "card" + (acc.active ? " active" : "");
  const head = document.createElement("div");
  head.className = "card-head";
  const name = document.createElement("span");
  name.className = "card-name";
  name.textContent = acc.login;
  head.appendChild(name);
  card.appendChild(head);
  if (!acc.active) {
    // 위젯 모드 더블클릭 전환 대상 — Rust가 provider "github"를 gh 통로로 보낸다
    card.classList.add("switchable");
    card.dataset.provider = "github";
    card.dataset.name = acc.login;
    const actions = document.createElement("div");
    actions.className = "card-actions";
    const switchBtn = document.createElement("button");
    switchBtn.className = "primary";
    switchBtn.textContent = t("switchBtn");
    switchBtn.addEventListener("click", async () => {
      switchBtn.disabled = true;
      try {
        await invoke("github_switch", { name: acc.login });
        await render({ immediate: true });
      } catch (error) {
        toast(String(error), true);
        switchBtn.disabled = false;
      }
    });
    actions.appendChild(switchBtn);
    card.appendChild(actions);
  }
  return card;
}

async function renderGithub(target: DocumentFragment) {
  const section = document.createElement("section");
  const heading = document.createElement("h2");
  heading.className = "section-title";
  heading.textContent = "GITHUB";
  section.appendChild(heading);
  try {
    const snap = await invoke<GithubSnapshot>("github_list");
    if (!snap.gh_found || snap.accounts.length === 0) {
      const hint = document.createElement("p");
      hint.className = "hint";
      hint.textContent = snap.gh_found ? t("ghNoAccounts") : t("ghNotFound");
      section.appendChild(hint);
    } else {
      for (const acc of snap.accounts) section.appendChild(githubCard(acc));
      const help = document.createElement("p");
      help.className = "hint";
      help.textContent = t("ghAddHint");
      section.appendChild(help);
    }
  } catch (error) {
    const err = document.createElement("p");
    err.className = "usage-error";
    err.textContent = String(error);
    section.appendChild(err);
  }
  target.appendChild(section);
}

/// 컴팩트의 GITHUB — 이름·활성·더블클릭 전환만, 계정이 없으면 섹션 생략
async function renderGithubCompact(target: DocumentFragment) {
  try {
    const snap = await invoke<GithubSnapshot>("github_list");
    if (!snap.gh_found || snap.accounts.length === 0) return;
    const section = document.createElement("section");
    const head = document.createElement("div");
    head.className = "compact-head";
    const name = document.createElement("span");
    name.textContent = "GITHUB";
    head.appendChild(name);
    section.appendChild(head);
    for (const acc of snap.accounts) {
      const card = document.createElement("div");
      card.className = "card compact-card" + (acc.active ? " active" : "");
      if (!acc.active) {
        card.classList.add("switchable");
        card.dataset.provider = "github";
        card.dataset.name = acc.login;
      }
      const cardHead = document.createElement("div");
      cardHead.className = "card-head";
      const login = document.createElement("span");
      login.className = "card-name";
      login.textContent = acc.login;
      cardHead.appendChild(login);
      card.appendChild(cardHead);
      section.appendChild(card);
    }
    target.appendChild(section);
  } catch {
    // 컴팩트는 표시 전용 — 조용히 넘긴다
  }
}

/// 컴팩트용 라벨 축약: "5 Hours"→5h, "Weekly"→wk, "Daily"→1d, Fable→fb, 그 외 모델→앞 2글자
function compactLabel(win: UsageWindow): string {
  const label = win.label;
  if (label === "5 Hours") return "5h";
  if (label === "Weekly") return "wk";
  if (label === "Daily") return "1d";
  if (label === "Fable") return "fb";
  const hours = label.match(/^(\d+) Hours?$/);
  if (hours) return `${hours[1]}h`;
  const days = label.match(/^(\d+) Days?$/);
  if (days) return `${days[1]}d`;
  return label.slice(0, 2).toLowerCase();
}

/// 컴팩트용 리셋 시간 — 콜론 개수가 단위 급을 나타낸다:
/// 24시간 이상이면 일::시(5::17), 그 밑이면 시:분(2:21)
function compactReset(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const ts = /^\d+$/.test(resetsAt) ? Number(resetsAt) * 1000 : Date.parse(resetsAt);
  if (Number.isNaN(ts)) return "";
  const diff = ts - Date.now();
  if (diff <= 0) return "0:00";
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (hours >= 24) return `${days}::${String(hours % 24).padStart(2, "0")}`;
  return `${hours}:${String(minutes % 60).padStart(2, "0")}`;
}

/// 컴팩트 카드 하나 — 이메일·구독 배지·사용량 요약. Type 2와 같은 카드 규칙
/// (.card/.active/.switchable)을 쓰므로 활성 색·채도·더블클릭 전환이 그대로 동작한다.
async function compactCard(provider: ProviderId, profile: ProfileInfo): Promise<HTMLElement> {
  const card = document.createElement("div");
  card.className = "card compact-card" + (profile.active ? " active" : "");
  if (!profile.active) {
    card.classList.add("switchable");
    card.dataset.provider = provider;
    card.dataset.name = profile.name;
  }

  const head = document.createElement("div");
  head.className = "card-head";
  const email = document.createElement("span");
  email.className = "card-name";
  email.textContent = profile.email ?? profile.name;
  email.title = t("profileNameTooltip", { name: profile.name });
  head.appendChild(email);
  if (profile.plan) {
    const plan = document.createElement("span");
    plan.className = "badge plan";
    plan.textContent = profile.plan;
    if (profile.plan_tier) {
      const tier = document.createElement("span");
      tier.className = `tier tier-${profile.plan_tier}`;
      tier.textContent = String(profile.plan_tier);
      plan.appendChild(tier);
    }
    head.appendChild(plan);
  }
  card.appendChild(head);

  try {
    const usage = await invoke<Usage>("fetch_usage", {
      provider,
      profile: profile.active ? null : profile.name,
    });
    if (usage.stale) {
      // 컴팩트에서도 이전 수치임을 숨기지 않는다 — 줄을 흐리고 머리에 나이를 붙인다
      card.classList.add("stale");
      const age = document.createElement("span");
      age.className = "c-stale";
      age.textContent = compactStaleAge(usage.stale_age_secs);
      head.appendChild(age);
    }
    for (const win of usage.windows) {
      const row = document.createElement("div");
      row.className = "compact-row";
      const label = document.createElement("span");
      label.className = "c-label";
      label.textContent = compactLabel(win);
      label.title = win.label;
      const bar = document.createElement("div");
      bar.className = "bar";
      const fill = document.createElement("div");
      fill.className = "bar-fill";
      if (win.percent >= 85) fill.classList.add("danger");
      else if (win.percent >= 60) fill.classList.add("warn");
      fill.style.width = `${Math.min(100, Math.max(0, win.percent))}%`;
      bar.appendChild(fill);
      const reset = document.createElement("span");
      reset.className = "c-reset";
      reset.textContent = compactReset(win.resets_at);
      reset.title = t("resetTooltip");
      row.append(label, bar, reset);
      card.appendChild(row);
    }
  } catch {
    // 컴팩트 모드에서는 조회 실패를 조용히 넘긴다 — 다음 주기에 다시 시도
  }
  return card;
}

/// 컴팩트 모드: Type 2의 축소판 — 모든 계정이 나오고 더블클릭 전환도 된다
async function renderProviderCompact(
  provider: ProviderId,
  title: string,
  target: DocumentFragment,
) {
  try {
    const snap = await invoke<Snapshot>("list_profiles", { provider });
    if (snap.profiles.length === 0) return; // 저장된 계정이 없으면 섹션 생략

    const section = document.createElement("section");
    const head = document.createElement("div");
    head.className = "compact-head";
    const name = document.createElement("span");
    name.textContent = title;
    head.appendChild(name);
    section.appendChild(head);

    // 카드를 병렬로 준비해 순서대로 붙인다 — 하나씩 기다리며 주루룩 생기지 않게
    const cards = await Promise.all(
      snap.profiles.map((profile) => compactCard(provider, profile)),
    );
    for (const card of cards) section.appendChild(card);
    target.appendChild(section);
  } catch {
    // 목록 실패도 조용히 — 컴팩트는 표시 전용
  }
}

let renderQueued = false;
/// 큐된 재요청 중 하나라도 즉시 렌더(상태 변경)였으면 다음 바퀴도 즉시로 돈다
let queuedImmediate = false;
/// 스무스 렌더의 사용량 대기를 즉시 끝내는 스위치 (상태 변경이 끼어들 때)
let renderAbort: (() => void) | null = null;

/// immediate: 전환·삭제·모드 변경처럼 "지금 상태가 바뀐" 렌더 — 새 목록을 바로
/// 보여주고 사용량은 교체된 카드에 이어서 채운다. 생략(스무스)은 주기·수동
/// 새로고침 — 기존 화면을 그대로 둔 채 다 받아진 뒤 한 번에 교체한다.
async function render(opts?: { immediate?: boolean }) {
  // 새로고침 연타·자동 주기와의 경합으로 화면이 겹쳐 그려지는 것을 막고,
  // 그리는 도중 재요청이 오면 끝난 뒤 한 번 더 그린다
  if (rendering) {
    renderQueued = true;
    if (opts?.immediate) queuedImmediate = true;
    // 진행 중인 스무스 대기는 낡은 버퍼를 기다리는 중 — 즉시 끝내고 다시 그리게
    renderAbort?.();
    return;
  }
  rendering = true;
  let thisImmediate = opts?.immediate ?? false;
  try {
    do {
      renderQueued = false;
      thisImmediate = thisImmediate || queuedImmediate;
      queuedImmediate = false;
      // 로그인 패널이 열린 채 다른 조작(전환·삭제·저장)으로 재렌더가 일어나면
      // 패널 DOM이 사라져 위젯이 영구 마비되던 문제 — 재렌더는 곧 로그인 흐름 포기로
      // 간주하고 백엔드 세션까지 정리한다 (red-review 2라운드)
      if (loginOpen) {
        loginOpen = false;
        void invoke("cancel_login");
      }
      // 그리는 도중 모드가 바뀌어도 한 화면은 단일 모드로 —
      // 프로바이더마다 다른 모드로 그려지는 혼종 화면 방지
      const mode = viewMode;
      // 화면을 지우고 처음부터 다시 그리면 새로고침마다 카드가 전부 사라졌다
      // 주루룩 돌아온다 — 보이지 않는 버퍼에 완성해 두고 한 번에 교체한다
      const buffer = document.createDocumentFragment();
      const pending: Promise<unknown>[] = [];
      for (const { id, title } of PROVIDERS) {
        if (mode === "compact") {
          await renderProviderCompact(id, title, buffer);
        } else {
          await renderProvider(id, title, buffer, pending);
        }
      }
      if (mode === "compact") {
        await renderGithubCompact(buffer);
      } else {
        await renderGithub(buffer);
      }
      if (!thisImmediate && app.childElementCount > 0 && !renderQueued) {
        // 스무스 새로고침: 기존 화면을 그대로 둔 채 사용량까지 받아진 뒤 교체한다.
        // 일반 모드의 사용량 채움에는 10초 상한 — 조회 하나가 매달려도 여기서 안
        // 굳고, 상한에 걸쳐 교체돼도 남은 조회는 같은 카드(동일 노드)를 이어서
        // 채운다. (컴팩트는 카드를 빌드 단계에서 만들므로 이 상한 밖 — 조회는
        // 캐시로 대부분 즉시다)
        await Promise.race([
          Promise.allSettled(pending),
          new Promise<void>((resolve) => window.setTimeout(resolve, 10_000)),
          new Promise<void>((resolve) => {
            renderAbort = resolve;
          }),
        ]);
        renderAbort = null;
      }
      // 그리는·기다리는 사이 상태가 바뀌었으면 이 버퍼는 낡았고(전환·삭제),
      // 화면에는 그새 생긴 로그인 패널·입력 중인 글자가 있을 수 있다 — 교체를
      // 접는다. 큐가 있으면 새 상태로 다시 그리고, 없으면 다음 주기에 맡긴다.
      if (renderQueued || userIsBusy()) continue;
      // 첫 화면은 뼈대를 먼저 보여주고 사용량은 채워지는 대로 붙는다
      app.replaceChildren(buffer);
      thisImmediate = false;
    } while (renderQueued);
  } finally {
    rendering = false;
    fitHeight();
  }
}

/// 자동 새로고침이 입력 중인 프로필 이름을 날리지 않게 한다
function userIsBusy(): boolean {
  const el = document.activeElement;
  const typing = el instanceof HTMLInputElement && el.value.trim().length > 0;
  return typing || loginOpen;
}

const appWindow = getCurrentWindow();

// 보기 모드 3단계 사이클: 일반 → 고정(사용량 위젯) → 컴팩트(활성 계정 요약만) → 일반
// 고정·컴팩트 공통: 조작 숨김, 클릭 투과, ☰ 핸들로만 이동. (항상-위는 창 기본 설정)
type ViewMode = "normal" | "locked" | "compact";
const lockBtn = document.getElementById("pin") as HTMLButtonElement;
let viewMode: ViewMode = (() => {
  const stored = localStorage.getItem("switcher.viewmode");
  if (stored === "locked" || stored === "compact") return stored;
  // 예전 저장값 이관
  return localStorage.getItem("switcher.locked") === "1" ? "locked" : "normal";
})();
// 파생: 위젯형(조작 숨김·투과) 여부
let locked = viewMode !== "normal";

function applyViewMode() {
  locked = viewMode !== "normal";
  app.classList.toggle("locked", locked);
  app.classList.toggle("compact", viewMode === "compact");
  // 타이틀바도 위젯 모드로 (이름·새로고침·슬라이더 숨김, 남은 버튼은 호버 시에만 또렷)
  document.body.classList.toggle("locked", locked);
  lockBtn.classList.toggle("pinned", locked);
  lockBtn.textContent =
    viewMode === "normal" ? "Type1" : viewMode === "locked" ? "Type2" : "Type3";
  // 위젯 모드에서는 ☰ 핸들을 잡아야만 창이 움직인다 — 타이틀바 전체 드래그를 끈다
  if (locked) {
    titlebarEl.removeAttribute("data-tauri-drag-region");
  } else {
    titlebarEl.setAttribute("data-tauri-drag-region", "");
  }
  // 위젯 모드에서는 카드·버튼 위가 아니면 마우스가 뒤 창으로 통과한다
  void invoke("set_click_through", { enabled: locked });
  reportHitRegions();
}

/// 고정 모드의 히트 영역 보고.
/// action이 있는 항목 = 더블클릭으로 전환하는 카드 (마우스는 투과),
/// action이 없는 항목 = 실제로 눌러야 하는 UI (버튼·이동 핸들).
/// Rust의 호버 신호(card-hover)가 이 배열의 인덱스로 오므로 요소 순서를 함께 보관한다.
let hitElements: HTMLElement[] = [];

function reportHitRegions() {
  hitElements = [];
  const regions: { rect: number[]; action: [string, string] | null }[] = [];
  if (locked) {
    document.querySelectorAll<HTMLElement>(".card.switchable").forEach((el) => {
      const r = el.getBoundingClientRect();
      regions.push({
        rect: [r.left, r.top, r.width, r.height],
        action: [el.dataset.provider ?? "", el.dataset.name ?? ""],
      });
      hitElements.push(el);
    });
    document.querySelectorAll<HTMLElement>(".tb-actions, #drag-handle").forEach((el) => {
      const r = el.getBoundingClientRect();
      regions.push({ rect: [r.left, r.top, r.width, r.height], action: null });
      hitElements.push(el);
    });
  }
  void invoke("set_hit_regions", { regions });
}

// Rust 폴링이 보내는 호버 신호 — 투과 중엔 웹뷰가 자체 hover를 못 받는다
void listen<number>("card-hover", (event) => {
  document
    .querySelectorAll(".card.hit-hover")
    .forEach((el) => el.classList.remove("hit-hover"));
  const idx = event.payload;
  if (idx >= 0) hitElements[idx]?.classList.add("hit-hover");
});

// 데모(GIF)용 플래그 — 켜지면 전환 완료 안내를 띄우지 않고 반투명하게 시작한다
let demoMode = false;
void invoke<boolean>("demo_mode").then((on) => {
  demoMode = on;
  if (on) applyAlpha(55);
});

// Rust가 실행한 더블클릭 전환의 결과
void listen<{ ok: boolean; provider?: string; name?: string; error?: string }>(
  "account-switched",
  (event) => {
    if (event.payload.ok) {
      const el = hitElements.find(
        (candidate) =>
          candidate.dataset.provider === event.payload.provider &&
          candidate.dataset.name === event.payload.name,
      );
      // 전환된 카드가 살짝 빛나고 나서 다시 그린다 — 성공 토스트는 띄우지 않는다
      el?.classList.add("switch-flash");
      window.setTimeout(() => void render({ immediate: true }), 380);
    } else if (!demoMode) {
      toast(event.payload.error ?? t("switchFailed"), true);
    }
  },
);

lockBtn.addEventListener("click", () => {
  viewMode = viewMode === "normal" ? "locked" : viewMode === "locked" ? "compact" : "normal";
  localStorage.setItem("switcher.viewmode", viewMode);
  applyViewMode();
  // 모드가 바뀌면 화면 구성이 달라진다 — 다시 그린다 (열려 있던 로그인 패널도 정리됨)
  void render({ immediate: true });
});
applyViewMode();

// 투명도 — 2단계 커브.
// 100%→50%: 배경 채움(--bg-alpha)만 1→0으로 빠지고 골조는 그대로.
// 50%→0%: 배경은 이미 없고, 골조(--fg-alpha)가 1→0.6으로 옅어진다.
const alphaSlider = document.getElementById("alpha") as HTMLInputElement;
function applyAlpha(percent: number) {
  const clamped = Math.min(100, Math.max(0, percent));
  let bg: number;
  let fg: number;
  if (clamped >= 50) {
    bg = (clamped - 50) / 50;
    fg = 1;
  } else {
    bg = 0;
    fg = 0.6 + 0.4 * (clamped / 50);
  }
  document.documentElement.style.setProperty("--bg-alpha", bg.toFixed(3));
  document.documentElement.style.setProperty("--fg-alpha", fg.toFixed(3));
  alphaSlider.value = String(clamped);
}
applyAlpha(Number(localStorage.getItem("switcher.alpha") ?? "100"));
alphaSlider.addEventListener("input", () => {
  applyAlpha(Number(alphaSlider.value));
  localStorage.setItem("switcher.alpha", alphaSlider.value);
});

// 세로 크기 — 창 높이를 콘텐츠에 맞춰 자동 조절한다.
// 계정이 없으면 그만큼 짧아지고, 늘어나면 화면 높이 90%까지 따라 늘어난다.
let fitTimer: number | undefined;
// 마지막으로 적용한 목표 폭 — 폭 전환 감지는 실측이 아니라 이 값으로 한다
let lastAppliedWidth = 0;

function fitHeight() {
  window.clearTimeout(fitTimer);
  fitTimer = window.setTimeout(() => {
    const last = app.lastElementChild as HTMLElement | null;
    const bottomPad = parseFloat(getComputedStyle(app).paddingBottom) || 14;
    const content = last
      ? last.getBoundingClientRect().bottom -
        app.getBoundingClientRect().top +
        app.scrollTop +
        bottomPad
      : 40;
    const total = titlebarEl.offsetHeight + content + 2; // 테두리
    const max = Math.floor(window.screen.availHeight * 0.9);
    const target = Math.round(Math.max(80, Math.min(total, max)));
    // 컴팩트 모드는 창 자체도 좁게
    const width = viewMode === "compact" ? 240 : 360;
    void (async () => {
      // 크기 조절 기준은 "오른쪽 상단" — 목표 폭이 실제로 바뀌는 전환에서만
      // 우측 가장자리를 고정한다. (바깥 크기에는 그림자가 포함되므로 실측 폭과
      // 목표 폭을 비교하면 매번 어긋나 창이 조금씩 밀리는 버그가 있었다)
      const widthChanging = lastAppliedWidth !== 0 && lastAppliedWidth !== width;
      let rightEdge = 0;
      let topY = 0;
      if (widthChanging) {
        try {
          const pos = await appWindow.outerPosition();
          const size = await appWindow.outerSize();
          rightEdge = pos.x + size.width;
          topY = pos.y;
        } catch {
          rightEdge = 0;
        }
      }
      await appWindow.setSize(new LogicalSize(width, target));
      if (widthChanging && rightEdge !== 0) {
        try {
          // 새 폭의 실제 바깥 크기(그림자 포함)로 우측 가장자리를 되살린다
          const newSize = await appWindow.outerSize();
          await appWindow.setPosition(new PhysicalPosition(rightEdge - newSize.width, topY));
        } catch {
          // 위치 보정 실패는 치명적이지 않다
        }
      }
      lastAppliedWidth = width;
      // 히트 영역은 반드시 창 크기 변경이 "끝난 뒤" 보고해야 한다 —
      // 즉시 보고하면 옛 폭 기준 좌표가 남아 버튼 위 클릭이 투과돼 버린다
      window.setTimeout(reportHitRegions, 80);
    })();
  }, 120);
}

// 크기 변경(모드 전환 등) 완료 시점의 백업 갱신 — 어떤 경로로 리사이즈되든 좌표를 다시 잡는다
let resizeReportTimer: number | undefined;
void appWindow.onResized(() => {
  window.clearTimeout(resizeReportTimer);
  resizeReportTimer = window.setTimeout(reportHitRegions, 100);
});

// 내용 높이가 바뀌는 지점(렌더 완료·사용량 로딩·고정 토글)에서 fitHeight()를 직접 부른다
// — app 요소 자체는 창 크기에 묶여 있어 관찰자로는 콘텐츠 변화를 못 잡는다
// 데모·스크린샷용 초기 모드 강제 (SWITCHER_VIEW 환경변수) —
// 진행 중인 첫 렌더가 있으면 render()가 큐잉해 단일 모드로 다시 그린다
void invoke<string | null>("initial_view_mode").then((mode) => {
  if (mode === "normal" || mode === "locked" || mode === "compact") {
    viewMode = mode;
    applyViewMode();
    void render({ immediate: true });
  }
});

document.getElementById("refresh")!.addEventListener("click", () => {
  if (loginOpen) {
    toast(t("refreshBusy"), true);
    return;
  }
  void render();
});
window.setInterval(() => {
  if (!userIsBusy()) void render();
}, 5 * 60 * 1000);

/// 정적 골격(타이틀바)의 문자열 — 렌더 밖 요소라 언어가 바뀔 때 직접 갈아 끼운다
function applyStaticText() {
  document.documentElement.lang = currentLang();
  const refreshBtn = document.getElementById("refresh")!;
  refreshBtn.textContent = t("refresh");
  refreshBtn.setAttribute("title", t("refreshTooltip"));
  document.getElementById("drag-handle")!.setAttribute("title", t("dragHandle"));
  document.getElementById("blackbtn")!.setAttribute("title", t("blackTooltip"));
  alphaSlider.title = t("alphaTooltip");
  lockBtn.title = t("typeTooltip");
}

// 블랙 모니터 — 모든 화면을 최상위 검은 막으로 (해제는 오버레이 쪽: 흔들기·ESC)
document.getElementById("blackbtn")!.addEventListener("click", () => {
  void invoke("black_on").catch((error) => toast(String(error), true));
});

// 실행 시 자동 업데이트 결과 — 교체는 이미 끝났고 다음 실행부터 새 버전이다
void listen<string>("update-ready", (event) => {
  toast(t("updateReady", { ver: event.payload }));
});

// 트레이(설정 → 언어)에서 바꾸면 Rust가 저장을 마친 뒤 알려온다
let langFromEvent = false;
void listen<string>("language-changed", (event) => {
  setLang(event.payload);
  langFromEvent = true;
  applyStaticText();
  // 로그인 패널이 열려 있으면 재렌더하지 않는다 — render()의 "재렌더 = 로그인 포기"
  // 정책이 진행 중 세션을 취소해 버린다. 본문은 패널이 닫힐 때(onExit) 새 언어로 그려진다.
  if (!loginOpen) void render({ immediate: true });
});

// 시작: 저장된 언어를 먼저 읽고 첫 렌더 — 한국어로 그렸다 갈아엎는 깜빡임을 피한다
void (async () => {
  try {
    const saved = await invoke<string>("get_language");
    // 조회 응답을 기다리는 사이 트레이 이벤트로 언어가 이미 정해졌으면 구값으로 덮지 않는다
    if (!langFromEvent) setLang(saved);
  } catch {
    // 설정을 못 읽으면 한국어로 진행
  }
  applyStaticText();
  void render();
})();
