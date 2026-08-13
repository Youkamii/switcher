import { invoke } from "@tauri-apps/api/core";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { currentLang, setLang, t } from "./i18n";
import {
  clampWindowToWorkArea,
  logicalWorkAreaHeight,
  monitorGeometryKey,
  type PhysicalRect,
} from "./windowGeometry";

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

// 빠른 모드 전환이 같은 계정 조회를 겹쳐 시작하지 않게 진행 중 요청을 공유한다.
// 완료되면 바로 비워 다음 수동 새로고침은 백엔드 캐시/정책에 따라 새로 판정한다.
const usageInflight = new Map<string, Promise<Usage>>();

function fetchUsageShared(
  provider: ProviderId,
  profile: string | null,
  accountId: string,
): Promise<Usage> {
  // 활성 조회의 profile은 항상 null이므로 계정 ID까지 키에 넣어야 전환 직후
  // 새 활성 카드가 이전 계정의 진행 중 요청을 이어받지 않는다.
  const key = JSON.stringify([provider, accountId]);
  const existing = usageInflight.get(key);
  if (existing) return existing;
  const request = invoke<Usage>("fetch_usage", { provider, profile });
  usageInflight.set(key, request);
  void request.then(
    () => {
      if (usageInflight.get(key) === request) usageInflight.delete(key);
    },
    () => {
      if (usageInflight.get(key) === request) usageInflight.delete(key);
    },
  );
  return request;
}

const PROVIDERS = [
  { id: "claude", title: "CLAUDE" },
  { id: "codex", title: "CODEX" },
] as const;

type ProviderId = (typeof PROVIDERS)[number]["id"];

const app = document.getElementById("app")!;
const titlebarEl = document.querySelector(".titlebar") as HTMLElement;
let rendering = false;
/// 로그인 패널이 열려 있으면 자동 새로고침이 화면을 갈아엎지 않게 한다
let loginOpen = false;
let loginSessionId: string | null = null;

/// 화면 알림(토스트)은 제거됐다 — 의미 없는 메시지가 위젯 폭에도 안 맞게
/// 떠서 없앰 (사용자 결정, 2026-08-07). 원인 추적을 위해 콘솔에만 남긴다.
function toast(message: string, isError = false) {
  if (isError) console.error(message);
  else console.log(message);
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

async function loadUsage(
  provider: ProviderId,
  card: HTMLElement,
  profile: string | null,
  accountId: string,
) {
  const box = document.createElement("div");
  box.className = "usage-box";
  const loading = document.createElement("div");
  loading.className = "usage-note";
  loading.textContent = t("loadingUsage");
  box.appendChild(loading);
  card.appendChild(box);

  try {
    const usage = await fetchUsageShared(provider, profile, accountId);
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
  if (profile.active && visibility.tfsd) {
    // 자율주행 중 표시 — 카드 배경 정중앙 워터마크 + 카드 호버 설명
    card.appendChild(tfsdWatermark());
    card.title = t("tfsdTooltip");
  }
  card.appendChild(head);

  // 활성 프로필은 활성 파일(항상 최신 토큰), 비활성은 보관함 토큰으로 조회.
  // 프라미스는 렌더러가 모은다 — 새로고침 때 다 받아진 뒤 한 번에 교체하기 위해
  pending.push(loadUsage(provider, card, profile.active ? null : profile.name, profile.id));

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
  session_id: string;
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
        const result = await invoke<LoginOutcome>("submit_login_code", {
          code,
          sessionId: prompt.session_id,
        });
        reportLogin(result);
      } catch (error) {
        const message = String(error);
        // "코드가 거부" = CLI가 몇 초 안에 거부를 알렸고(백엔드 화면 감지, 실측)
        // 세션은 재입력을 기다리고 있다 — 패널을 유지하고 같은 자리에서 다시 받는다
        if (message.includes("코드가 거부")) {
          toast(message, true);
          okBtn.disabled = false;
          input.disabled = false;
          okBtn.textContent = t("ok");
          input.select();
          return;
        }
        // 그 외 실패는 세션이 이미 끝난 상태라 재시도가 불가능하다 —
        // 패널을 닫고 처음부터 다시 시작하게 안내한다
        toast(t("retryFromStart", { error: message }), true);
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
        const result = await invoke<LoginOutcome>("await_device_login", {
          sessionId: prompt.session_id,
        });
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
    void invoke("cancel_login", { sessionId: prompt.session_id });
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
    // 주소를 받는 수 초 동안 주기 렌더가 DOM을 갈아엎으면 패널이 분리된 노드에
    // 붙어 영영 안 보인다 — 시작 전에 loginOpen을 올려 렌더를 막는다 (red-review)
    loginOpen = true;
    loginSessionId = null;
    try {
      const prompt = await invoke<LoginPrompt>("start_login", { provider });
      if (!loginOpen) {
        void invoke("cancel_login", { sessionId: prompt.session_id });
        return;
      }
      loginSessionId = prompt.session_id;
      addBtn.hidden = true;
      slot.appendChild(
        loginPanel(prompt, () => {
          // 취소된 이전 waiter가 늦게 끝나도 그 사이 시작한 새 로그인 패널을
          // 닫거나 재렌더로 지우지 못하게 현재 세션 콜백만 받는다.
          if (loginSessionId !== prompt.session_id) return;
          loginSessionId = null;
          loginOpen = false;
          void render({ immediate: true });
        }),
      );
    } catch (error) {
      loginSessionId = null;
      loginOpen = false;
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
    try {
      // 빈 이름이면 백엔드가 auto_name으로 자동 작명한다 (#18 UX) —
      // 실제 저장된 이름이 돌아오므로 안내에 그대로 쓴다
      const saved = await invoke<string>("save_profile", { provider, name });
      toast(t("saveDone", { name: saved }));
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

// ── 접이식 섹션 (DISPLAY·GITHUB) ─────────────────────────────────
// 사용량처럼 상시 감시할 정보가 아니라 기본 접힘 — 제목 클릭으로 펼치고,
// 하나를 펼치면 다른 하나는 접히며(아코디언), 섹션 밖 클릭에 다시 접힌다.
// 접힌 동안엔 데이터 조회(gh 목록·DDC 읽기)도 건너뛰어 렌더가 가볍다.
type CollapsibleKey = "github" | "display";
const expanded: Record<CollapsibleKey, boolean> = { github: false, display: false };

function toggleSection(key: CollapsibleKey) {
  // 로그인 패널이 열려 있으면 토글하지 않는다 — 재렌더가 세션을 취소해 버린다 (red-review)
  if (loginOpen) {
    toast(t("loginBusy"), true);
    return;
  }
  const next = !expanded[key];
  expanded.github = false;
  expanded.display = false;
  expanded[key] = next;
  void render({ immediate: true });
}

/// 바깥 클릭 시 펼쳐진 섹션만 **외과적으로** 접는다 — 전체 재렌더는 입력 중 텍스트·
/// 삭제 확인 무장·로그인 패널을 날리는 부작용이 있어 쓰지 않는다 (red-review).
/// bubble 단계라 클릭 대상의 원래 핸들러가 먼저 실행된다. 위젯 모드에서는 바깥
/// 클릭이 뒤 창으로 투과되므로 제목 재클릭·다른 섹션 펼치기로 접는다.
function collapseSectionsInPlace() {
  expanded.github = false;
  expanded.display = false;
  document.querySelectorAll<HTMLElement>("section[data-collapsible]").forEach((el) => {
    const key = el.dataset.collapsible as CollapsibleKey;
    el.replaceChildren(
      collapsibleHeader(key === "github" ? "GITHUB" : "DISPLAY", key, viewMode === "compact"),
    );
    // 새로 만든 머리글에 드래그 시작을 다시 붙인다 — 안 하면 다음 전체
    // 재렌더까지 이 섹션만 드래그가 안 된다 (red-review)
    if (viewMode === "normal") attachHeaderDrag(el, key);
  });
  fitHeight(); // 줄어든 높이 반영 — 완료 시 히트 영역도 다시 보고된다
}

document.addEventListener("click", (event) => {
  if (!expanded.github && !expanded.display) return;
  if (loginOpen) return; // 로그인 중엔 접지 않는다 — 패널·세션 보호
  const target = event.target as HTMLElement | null;
  if (target?.closest("section[data-collapsible]")) return;
  collapseSectionsInPlace();
});

/// 접이식 섹션 토글 — 가로를 채우는 버튼(왼쪽 제목, 오른쪽 ▸/▾). 누르면 아래로 열린다.
/// 텍스트 제목은 오른쪽이 텅 비어 어정쩡했다 (사용자 피드백)
function collapsibleHeader(
  title: string,
  key: CollapsibleKey,
  compact: boolean,
): HTMLElement {
  const button = document.createElement("button");
  button.className = "section-toggle collapsible" + (compact ? " compact-toggle" : "");
  const label = document.createElement("span");
  label.textContent = title;
  const chev = document.createElement("span");
  chev.className = "chev";
  chev.textContent = expanded[key] ? "▾" : "▸";
  button.append(label, chev);
  button.addEventListener("click", () => toggleSection(key));
  return button;
}

/// 표시 기능 (트레이 설정 → 표시 기능) — 끈 섹션·버튼은 그리지 않는다
type Visibility = {
  claude: boolean;
  codex: boolean;
  github: boolean;
  black: boolean;
  display: boolean;
  tfsd: boolean;
};
let visibility: Visibility = {
  claude: true,
  codex: true,
  github: true,
  black: true,
  display: true,
  tfsd: false,
};

/// TFSD 워터마크 — 자율주행 중인 활성 카드의 배경 정중앙에 은은한 T.
/// pointer-events 없음(클릭·호버 방해 금지) — 설명 툴팁은 카드 자체에 단다.
/// 로고 패스는 simple-icons의 Tesla 아이콘(CC0) — DOM으로 조립 (innerHTML 금지 관례 유지)
function tfsdWatermark(): HTMLElement {
  const badge = document.createElement("span");
  badge.className = "tfsd-watermark";
  const svgNS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const logo = document.createElementNS(svgNS, "path");
  logo.setAttribute(
    "d",
    "M12 5.362l2.475-3.026s4.245.09 8.471 2.054c-1.082 1.636-3.231 2.438-3.231 2.438-.146-1.439-1.154-1.79-4.354-1.79L12 24 8.619 5.034c-3.18 0-4.188.354-4.335 1.792 0 0-2.146-.795-3.229-2.43C5.28 2.431 9.525 2.34 9.525 2.34L12 5.362l-.004.002H12v-.002zm0-3.899c3.415-.03 7.326.528 11.328 2.28.535-.968.672-1.395.672-1.395C19.625.612 15.528.015 12 0 8.472.015 4.375.61 0 2.349c0 0 .195.525.672 1.396C4.674 1.989 8.585 1.435 12 1.46v.003z",
  );
  logo.setAttribute("fill", "currentColor");
  svg.appendChild(logo);
  badge.appendChild(svg);
  return badge;
}

async function loadVisibility() {
  try {
    visibility = await invoke<Visibility>("get_visibility");
  } catch {
    // 설정을 못 읽으면 전부 표시
  }
  // 표시 기능에서 꺼진 섹션의 펼침 상태는 정리한다 — 스테일 플래그가 남으면
  // 다음 아무 클릭이 무의미한 접힘 경로를 타는 사고가 있었다 (red-review)
  if (!visibility.github) expanded.github = false;
  if (!visibility.display) expanded.display = false;
  (document.getElementById("blackbtn") as HTMLElement).style.display =
    visibility.black ? "" : "none";
  // 🚗 버튼의 켜짐 표시는 항상 설정과 동기 — 트레이에서 토글해도 다음
  // 렌더에서 따라온다 (T 배지와 같은 원천: visibility.tfsd)
  document.getElementById("tfsdbtn")?.classList.toggle("pinned", visibility.tfsd);
}

/// GITHUB 계정 카드 — 사용량 없음: 이름·활성 표시·전환뿐. 컴팩트는 전환 버튼 생략.
/// 토큰은 위젯이 만지지 않는다 (gh가 keyring에 관리, 전환은 gh auth switch 대행)
function githubCard(acc: GithubAccount, compact = false): HTMLElement {
  const card = document.createElement("div");
  card.className = (compact ? "card compact-card" : "card") + (acc.active ? " active" : "");
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
    if (!compact) {
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
  }
  return card;
}

async function renderGithub(target: DocumentFragment) {
  const section = document.createElement("section");
  section.dataset.collapsible = "github";
  section.appendChild(collapsibleHeader("GITHUB", "github", false));
  if (!expanded.github) {
    target.appendChild(section);
    return;
  }
  try {
    const snap = await invoke<GithubSnapshot>("github_list");
    if (!snap.gh_found) {
      const hint = document.createElement("p");
      hint.className = "section-note";
      hint.textContent = t("ghNotFound");
      section.appendChild(hint);
    } else {
      if (snap.accounts.length === 0) {
        const hint = document.createElement("p");
        hint.className = "section-note";
        hint.textContent = t("ghNoAccounts");
        section.appendChild(hint);
      } else {
        for (const acc of snap.accounts) section.appendChild(githubCard(acc));
      }
      githubAddButton(section);
    }
  } catch (error) {
    const err = document.createElement("p");
    err.className = "usage-error";
    err.textContent = String(error);
    section.appendChild(err);
  }
  target.appendChild(section);
}

/// GITHUB 계정 추가 — 코덱스와 같은 장치 코드 UX (주소 + 일회용 코드 → 브라우저 입력)
function githubAddButton(section: HTMLElement) {
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
    // 시작 대기 중 재렌더가 패널을 분리된 DOM에 붙이는 사고 방지 (위 addAccountButton과 동일)
    loginOpen = true;
    try {
      const prompt = await invoke<{ url: string; device_code: string }>("github_login_start");
      addBtn.hidden = true;
      const onExit = () => {
        loginOpen = false;
        void render({ immediate: true });
      };
      const panel = document.createElement("div");
      panel.className = "login-panel";
      const steps = document.createElement("div");
      steps.className = "help";
      steps.textContent = t("stepsCodex");
      panel.appendChild(steps);
      panel.appendChild(copyBox(t("loginUrl"), prompt.url, false));
      panel.appendChild(copyBox(t("oneTimeCode"), prompt.device_code, true));
      const waiting = document.createElement("div");
      waiting.className = "usage-note";
      waiting.textContent = t("waitingBrowser");
      panel.appendChild(waiting);
      void (async () => {
        try {
          const login = await invoke<string>("github_login_wait");
          toast(t("ghAdded", { login }));
        } catch (error) {
          toast(String(error), true);
        }
        onExit();
      })();
      const cancelBtn = document.createElement("button");
      cancelBtn.className = "link";
      cancelBtn.textContent = t("cancel");
      cancelBtn.addEventListener("click", () => {
        void invoke("github_login_cancel");
        onExit();
      });
      panel.appendChild(cancelBtn);
      slot.appendChild(panel);
    } catch (error) {
      loginOpen = false;
      toast(String(error), true);
      addBtn.disabled = false;
      addBtn.textContent = t("addAccount");
    }
  });
}

type DisplayInfo = { id: number; name: string; brightness: number | null };

/// DISPLAY 섹션 — 모니터별 밝기 슬라이더 (DDC/CI 실제 백라이트 명령).
/// 모든 모니터를 카드 하나에 모니터당 한 줄(번호·슬라이더·%)로 — 모니터마다
/// 카드·이름 헤더를 두던 이전 구조는 세로 여백이 과했다. 전체 이름은 번호 툴팁에.
/// Type 2/3(클릭 투과)에서도 조작된다 — reportHitRegions가 줄을 히트 영역으로 보고.
async function renderDisplays(target: DocumentFragment, compact: boolean) {
  const section = document.createElement("section");
  section.dataset.collapsible = "display";
  section.appendChild(collapsibleHeader("DISPLAY", "display", compact));
  if (!expanded.display) {
    target.appendChild(section);
    return;
  }
  try {
    const monitors = await invoke<DisplayInfo[]>("display_list");
    if (monitors.length === 0) {
      // 펼쳤는데 모니터가 없으면(미지원 플랫폼 등) 안내만
      const note = document.createElement("p");
      note.className = "section-note";
      note.textContent = t("dspUnsupported");
      section.appendChild(note);
    } else {
      const card = document.createElement("div");
      card.className = compact ? "card compact-card" : "card";
      for (const monitor of monitors) {
        const row = document.createElement("div");
        row.className = "display-row";
        const label = document.createElement("span");
        label.className = "display-name";
        label.textContent = String(monitor.id + 1);
        label.title = monitor.name;
        row.appendChild(label);
        if (monitor.brightness == null) {
          const note = document.createElement("span");
          note.className = "display-na";
          note.textContent = t("dspUnsupported");
          note.title = t("dspUnsupported");
          row.appendChild(note);
        } else {
          const slider = document.createElement("input");
          slider.type = "range";
          slider.min = "0";
          slider.max = "100";
          slider.step = "1";
          slider.value = String(monitor.brightness);
          const pct = document.createElement("span");
          pct.className = "display-pct";
          pct.textContent = `${monitor.brightness}%`;
          // 밝기 명령은 모니터마다 수십~수백 ms — 드래그 중엔 표시만 갱신하고
          // 손을 잠깐 멈추면 마지막 값 하나만 보낸다
          let debounce: number | undefined;
          slider.addEventListener("input", () => {
            pct.textContent = `${slider.value}%`;
            window.clearTimeout(debounce);
            debounce = window.setTimeout(() => {
              // name을 함께 보내 목록 이후 모니터 구성이 바뀐 경우 엉뚱한 모니터에
              // 쓰지 않게 한다 (Rust가 대조 후 불일치면 에러)
              void invoke("display_set_brightness", {
                id: monitor.id,
                percent: Number(slider.value),
                name: monitor.name,
              }).catch((error) => toast(String(error), true));
            }, 250);
          });
          row.append(slider, pct);
        }
        card.appendChild(row);
      }
      section.appendChild(card);
    }
  } catch {
    // 조회 실패 — 제목까지 증발시키지 않고 안내만 남긴다
    const note = document.createElement("p");
    note.className = "section-note";
    note.textContent = t("dspUnsupported");
    section.appendChild(note);
  }
  target.appendChild(section);
}

/// 컴팩트의 GITHUB — 이름·활성·더블클릭 전환만, 계정이 없으면 섹션 생략
async function renderGithubCompact(target: DocumentFragment) {
  const section = document.createElement("section");
  section.dataset.collapsible = "github";
  section.appendChild(collapsibleHeader("GITHUB", "github", true));
  if (!expanded.github) {
    target.appendChild(section);
    return;
  }
  try {
    const snap = await invoke<GithubSnapshot>("github_list");
    if (!snap.gh_found || snap.accounts.length === 0) {
      const hint = document.createElement("p");
      hint.className = "section-note";
      hint.textContent = snap.gh_found ? t("ghNoAccounts") : t("ghNotFound");
      section.appendChild(hint);
    } else {
      for (const acc of snap.accounts) section.appendChild(githubCard(acc, true));
    }
  } catch {
    // 컴팩트는 표시 전용 — 조용히 넘긴다
  }
  target.appendChild(section);
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

/// 미니멀용 초약자 라벨 — 5 Hours→5, Weekly→W, Fable→F. 그 외는 첫 글자 (#41)
function minimalLabel(win: UsageWindow): string {
  const label = win.label;
  if (label === "5 Hours") return "5";
  if (label === "Weekly") return "W";
  if (label === "Fable") return "F";
  const hours = label.match(/^(\d+) Hours?$/);
  if (hours) return hours[1];
  return label.charAt(0).toUpperCase();
}

/// 컴팩트 카드 하나 — 이메일·구독 배지·사용량 요약. Type 1과 같은 카드 규칙
/// (.card/.active/.switchable)을 쓰므로 활성 색·채도·더블클릭 전환이 그대로 동작한다.
/// minimal(Type 3)이면 머리(이메일·플랜·나이)와 리셋 시각을 빼고 라벨+바만 남긴다.
function compactCard(
  provider: ProviderId,
  profile: ProfileInfo,
  minimal: boolean,
  pending: Promise<unknown>[],
): HTMLElement {
  const card = document.createElement("div");
  card.className = "card compact-card" + (profile.active ? " active" : "");
  // 미니멀의 프로바이더 구분은 왼쪽 색 스트라이프가 맡는다 (#41 후속)
  card.classList.add(`prov-${provider}`);
  if (!profile.active) {
    card.classList.add("switchable");
    card.dataset.provider = provider;
    card.dataset.name = profile.name;
  }

  const head = document.createElement("div");
  if (!minimal) {
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
  }
  if (profile.active && visibility.tfsd) {
    card.appendChild(tfsdWatermark());
    card.title = t("tfsdTooltip");
  }

  const load = (async () => {
    try {
      const usage = await fetchUsageShared(
        provider,
        profile.active ? null : profile.name,
        profile.id,
      );
      if (usage.stale) {
        // 컴팩트에서도 이전 수치임을 숨기지 않는다 — 줄을 흐리고 머리에 나이를 붙인다
        // (미니멀은 붙일 머리가 없으니 줄 흐림만 남는다)
        card.classList.add("stale");
        if (!minimal) {
          const age = document.createElement("span");
          age.className = "c-stale";
          age.textContent = compactStaleAge(usage.stale_age_secs);
          head.appendChild(age);
        }
      }
      for (const win of usage.windows) {
        const row = document.createElement("div");
        row.className = "compact-row";
        const label = document.createElement("span");
        label.className = "c-label";
        label.textContent = minimal ? minimalLabel(win) : compactLabel(win);
        label.title = win.label;
        const bar = document.createElement("div");
        bar.className = "bar";
        const fill = document.createElement("div");
        fill.className = "bar-fill";
        if (win.percent >= 85) fill.classList.add("danger");
        else if (win.percent >= 60) fill.classList.add("warn");
        fill.style.width = `${Math.min(100, Math.max(0, win.percent))}%`;
        bar.appendChild(fill);
        // 사용량 % 숫자를 바 위에 겹친다 (사용자 지시 2회: 처음엔 남은 한도였으나
        // 100% 사용이 "0"으로 보여 헷갈린다 — 사용한 양으로 정정). 바 두께는 절대
        // 건드리지 않는다 (바를 키웠다가 질책받음). 바 안은 overflow:hidden이라
        // 잘리므로 래퍼에 얹어 글자가 바 위아래로 걸치게 한다
        const wrap = document.createElement("div");
        wrap.className = "bar-wrap";
        wrap.appendChild(bar);
        const used = document.createElement("span");
        used.className = "bar-num";
        used.textContent = String(Math.min(100, Math.max(0, Math.round(win.percent))));
        wrap.appendChild(used);
        row.append(label, wrap);
        if (!minimal) {
          const reset = document.createElement("span");
          reset.className = "c-reset";
          reset.textContent = compactReset(win.resets_at);
          reset.title = t("resetTooltip");
          row.appendChild(reset);
        }
        card.appendChild(row);
      }
    } catch {
      // 컴팩트 모드에서는 조회 실패를 조용히 넘긴다 — 다음 주기에 다시 시도
    } finally {
      // 즉시 모드에서는 카드 골격을 먼저 붙인다. 사용량이 나중에 늘린 실제 높이를
      // 다시 맞추되, 이미 폐기된 렌더 버퍼는 창 크기에 영향을 주지 않게 한다.
      if (card.isConnected) fitHeight();
    }
  })();
  pending.push(load);
  return card;
}

/// 컴팩트(Type 2)·미니멀(Type 3) 렌더 — 모든 계정이 나오고 더블클릭 전환도 된다
async function renderProviderCompact(
  provider: ProviderId,
  title: string,
  target: DocumentFragment,
  minimal: boolean,
  pending: Promise<unknown>[],
) {
  try {
    const snap = await invoke<Snapshot>("list_profiles", { provider });
    if (snap.profiles.length === 0) return; // 저장된 계정이 없으면 섹션 생략

    const section = document.createElement("section");
    // 미니멀은 섹션 제목(CLAUDE/CODEX)도 없다 — 카드만 쌓인다 (#41 후속)
    if (!minimal) {
      const head = document.createElement("div");
      head.className = "compact-head";
      const name = document.createElement("span");
      name.textContent = title;
      head.appendChild(name);
      section.appendChild(head);
    }

    // 카드 골격은 즉시 붙이고 사용량만 뒤에서 병렬로 채운다. 모드 전환이 네트워크
    // 조회를 기다리며 멎지 않게 하면서, 일반 새로고침은 pending을 기다려 한 번에 바뀐다.
    const cards = snap.profiles.map((profile) => compactCard(provider, profile, minimal, pending));
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
        const sessionId = loginSessionId;
        loginSessionId = null;
        if (sessionId) void invoke("cancel_login", { sessionId });
        // gh 로그인 세션도 같은 정책 — 안 열려 있으면 무해한 no-op
        void invoke("github_login_cancel");
      }
      // 그리는 도중 모드가 바뀌어도 한 화면은 단일 모드로 —
      // 프로바이더마다 다른 모드로 그려지는 혼종 화면 방지
      const mode = viewMode;
      // 화면을 지우고 처음부터 다시 그리면 새로고침마다 카드가 전부 사라졌다
      // 주루룩 돌아온다 — 보이지 않는 버퍼에 완성해 두고 한 번에 교체한다
      const buffer = document.createDocumentFragment();
      const pending: Promise<unknown>[] = [];
      // 섹션은 사용자가 정한 순서(sectionOrder)대로 그린다 — Type1에서 머리글
      // 드래그로 바꾸고, 컴팩트에도 같은 순서가 적용된다.
      // 미니멀은 사용량 전용(#41)이라 GITHUB·DISPLAY는 그리지 않는다 —
      // 단 SYSTEM은 예외로 함께 나온다 (사용자 요청: PC 상태는 미니멀에서도).
      for (const key of sectionOrder) {
        const before = buffer.lastElementChild;
        if (key === "claude" || key === "codex") {
          if (!visibility[key]) continue;
          const title = PROVIDERS.find((p) => p.id === key)!.title;
          if (mode !== "normal") {
            await renderProviderCompact(key, title, buffer, mode === "minimal", pending);
          } else {
            await renderProvider(key, title, buffer, pending);
          }
        } else if (key === "github") {
          if (!visibility.github || mode === "minimal") continue;
          if (mode === "compact") {
            await renderGithubCompact(buffer);
          } else {
            await renderGithub(buffer);
          }
        } else if (key === "display") {
          if (!visibility.display || mode === "minimal") continue;
          await renderDisplays(buffer, mode === "compact");
        } else if (key === "system") {
          if (!monitorOn) continue;
          renderMonitor(buffer);
        }
        // 방금 붙은 섹션에 순서 키를 달고 Type1이면 드래그 이동을 붙인다
        // (렌더 함수가 아무것도 안 붙였을 수 있어 lastElementChild 변화로 판별)
        const added = buffer.lastElementChild as HTMLElement | null;
        if (added && added !== before && added.tagName === "SECTION") {
          added.dataset.key = key;
          enableSectionDrag(added, key, mode);
        }
      }
      if (!thisImmediate && app.childElementCount > 0 && !renderQueued) {
        // 스무스 새로고침: 기존 화면을 그대로 둔 채 사용량까지 받아진 뒤 교체한다.
        // 일반 모드의 사용량 채움에는 10초 상한 — 조회 하나가 매달려도 여기서 안
        // 굳고, 상한에 걸쳐 교체돼도 남은 조회는 같은 카드(동일 노드)를 이어서
        // 채운다. 컴팩트 사용량도 같은 pending 경로를 써서 캐시 적중 여부와 무관하다.
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
      // 새 SYSTEM 스켈레톤을 마지막 샘플로 즉시 채운다 — 스무스 교체마다
      // 이 섹션만 '--'로 깜빡이던 문제 (red-review). 다음 틱이 이어받는다
      if (monitorOn && monLastStats) {
        paintMonitor(monLastStats);
        drawMonSpark();
      }
      thisImmediate = false;
    } while (renderQueued);
  } finally {
    rendering = false;
    fitHeight();
  }
}

/// 자동 새로고침이 입력 중인 프로필 이름을 날리지 않게 한다.
/// 텍스트 입력만 본다 — range 슬라이더(투명도·밝기)는 value가 항상 차 있어
/// 포커스가 남으면 새로고침이 영구 정지하는 오인이 있었다 (red-review)
function userIsBusy(): boolean {
  const el = document.activeElement;
  const typing =
    el instanceof HTMLInputElement && el.type === "text" && el.value.trim().length > 0;
  // 섹션 드래그 중에도 스왑을 미룬다 — 잡고 있는 드래그가 소리 없이 죽지 않게.
  // dragend 유실로 고착된 래치는 pointermove 복구가 푼다 (리뷰 #53:
  // dragend 유실 시 렌더가 영구 차단되던 문제)
  return typing || loginOpen || dragKey !== null;
}

const appWindow = getCurrentWindow();

// 보기 모드 3단계 사이클: 일반 → 고정(사용량 위젯) → 컴팩트(활성 계정 요약만) → 일반
// 고정·컴팩트 공통: 조작 숨김, 클릭 투과, ☰ 핸들로만 이동. (항상-위는 창 기본 설정)
type ViewMode = "normal" | "compact" | "minimal";
const lockBtn = document.getElementById("pin") as HTMLButtonElement;
let viewMode: ViewMode = (() => {
  const stored = localStorage.getItem("switcher.viewmode");
  if (stored === "compact" || stored === "minimal") return stored;
  // Type2(위젯 풀형)는 폐지됐다(#41) — 예전 locked 저장값은 컴팩트로 이관
  if (stored === "locked") return "compact";
  return localStorage.getItem("switcher.locked") === "1" ? "compact" : "normal";
})();
// 파생: 위젯형(조작 숨김·투과) 여부
let locked = viewMode !== "normal";

function applyViewMode() {
  locked = viewMode !== "normal";
  app.classList.toggle("locked", locked);
  // 컴팩트·미니멀은 같은 축소 레이아웃을 공유하고, 미니멀이 위에 더 덜어낸다
  app.classList.toggle("compact", locked);
  app.classList.toggle("minimal", viewMode === "minimal");
  // 타이틀바도 위젯 모드로 (이름·새로고침·슬라이더 숨김, 남은 버튼은 호버 시에만 또렷)
  document.body.classList.toggle("locked", locked);
  document.body.classList.toggle("minimal", viewMode === "minimal");
  lockBtn.classList.toggle("pinned", locked);
  lockBtn.textContent =
    viewMode === "normal" ? "Type1" : viewMode === "compact" ? "Type2" : "Type3";
  // 위젯 모드에서는 ☰ 핸들을 잡아야만 창이 움직인다 — 타이틀바 전체 드래그를 끈다
  if (locked) {
    titlebarEl.removeAttribute("data-tauri-drag-region");
  } else {
    titlebarEl.setAttribute("data-tauri-drag-region", "");
  }
  // 위젯 모드에서는 카드·버튼 위가 아니면 마우스가 뒤 창으로 통과한다
  void invoke("set_click_through", { enabled: locked });
  refreshHitRegionsAfterLayout();
}

/// 고정 모드의 히트 영역 보고.
/// action이 있는 항목 = 더블클릭으로 전환하는 카드 (마우스는 투과),
/// action이 없는 항목 = 실제로 눌러야 하는 UI (버튼·이동 핸들).
/// Rust의 호버 신호(card-hover)가 이 배열의 인덱스로 오므로 요소 순서를 함께 보관한다.
let hitElements: HTMLElement[] = [];
let hitRegionRetryTimer: number | undefined;

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
    // .display-row: 펼쳐진 밝기 슬라이더 조작용 (접힘 상태면 DOM에 없다)
    // .collapsible: 접이식 섹션 제목 — 위젯 모드에서도 클릭해 펼칠 수 있게
    // .tb-actions는 컨테이너가 아니라 **자식을 하나씩** 보고한다 — Type3(미니멀)
    // 타이틀바가 컨테이너를 display:contents(박스 없음 → rect 0×0)로 만들어
    // 버튼 전체가 클릭 투과에 삼켜졌다 (사용자 보고: 타입3 버튼 무반응).
    // 숨김(0크기) 요소는 거른다 — 좌상단 유령 히트 방지.
    document
      .querySelectorAll<HTMLElement>(".tb-actions > *, #drag-handle, .display-row, .collapsible")
      .forEach((el) => {
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) return;
        regions.push({ rect: [r.left, r.top, r.width, r.height], action: null });
        hitElements.push(el);
      });
  }
  void invoke("set_hit_regions", { regions });
}

// 창 크기·배율 변경 직후에는 WebView 레이아웃이 한 박자 늦을 수 있다. 즉시값으로
// 클릭을 먼저 살리고, 실제 레이아웃이 끝난 뒤 한 번 더 보고해 오래된 좌표를 없앤다.
function refreshHitRegionsAfterLayout() {
  reportHitRegions();
  window.clearTimeout(hitRegionRetryTimer);
  hitRegionRetryTimer = window.setTimeout(reportHitRegions, 120);
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
  viewMode = viewMode === "normal" ? "compact" : viewMode === "compact" ? "minimal" : "normal";
  localStorage.setItem("switcher.viewmode", viewMode);
  applyViewMode();
  // 새 모드 CSS가 줄인 현재 내용만으로도 창을 먼저 따라붙인다. 새 카드 목록은
  // 바로 뒤의 즉시 렌더가 교체하고, 완성 높이는 그 렌더가 한 번 더 맞춘다.
  fitHeight();
  // 모드가 바뀌면 화면 구성이 달라진다 — 다시 그린다 (열려 있던 로그인 패널도 정리됨)
  void render({ immediate: true });
});
applyViewMode();

// 투명도 — 2단계 커브.
// 100%→50%: 배경 채움(--bg-alpha)만 1→0으로 빠지고 골조는 그대로.
// 50%→0%: 배경은 이미 없고, 글자·테두리(--fg-alpha)는 1→0.05로 사실상
// 사라지며, 사용량 바(--bar-alpha)도 1→0.45로 은은해진다 — "바까지 더
// 투명하게"가 사용자 의도 (#42 후속 재수정). 타입 버튼·이동 핸들만 예외.
const alphaSlider = document.getElementById("alpha") as HTMLInputElement;
function applyAlpha(percent: number) {
  const clamped = Math.min(100, Math.max(0, percent));
  let bg: number;
  let fg: number;
  let bar: number;
  if (clamped >= 50) {
    bg = (clamped - 50) / 50;
    fg = 1;
    bar = 1;
  } else {
    bg = 0;
    fg = 0.05 + 0.95 * (clamped / 50);
    bar = 0.45 + 0.55 * (clamped / 50);
  }
  document.documentElement.style.setProperty("--bg-alpha", bg.toFixed(3));
  document.documentElement.style.setProperty("--fg-alpha", fg.toFixed(3));
  document.documentElement.style.setProperty("--bar-alpha", bar.toFixed(3));
  alphaSlider.value = String(clamped);
}
applyAlpha(Number(localStorage.getItem("switcher.alpha") ?? "100"));
alphaSlider.addEventListener("input", () => {
  applyAlpha(Number(alphaSlider.value));
  localStorage.setItem("switcher.alpha", alphaSlider.value);
});

// 세로 크기 — 창 높이를 콘텐츠에 맞춰 자동 조절한다.
// 계정이 없으면 그만큼 짧아지고, 늘어나면 화면 높이 90%까지 따라 늘어난다.
let fitRevision = 0;
let fitQueued = false;
let fitRunning = false;
// 마지막으로 적용한 목표 폭 — 폭 전환 감지는 실측이 아니라 이 값으로 한다
let lastAppliedWidth = 0;
let currentWorkArea: PhysicalRect | null = null;
let currentWorkAreaHeight: number | null = null;
let currentMonitorKey = "";
let monitorSyncRevision = 0;
let monitorMoveTimer: number | undefined;
let monitorMoveForce = false;

function scheduleMonitorSync(force = false) {
  monitorMoveForce ||= force;
  window.clearTimeout(monitorMoveTimer);
  monitorMoveTimer = window.setTimeout(() => {
    monitorMoveTimer = undefined;
    const forceSync = monitorMoveForce;
    monitorMoveForce = false;
    void syncCurrentMonitor(forceSync);
  }, 180);
}

async function syncCurrentMonitor(force = false) {
  const revision = ++monitorSyncRevision;
  try {
    const monitor = await currentMonitor();
    if (!monitor || revision !== monitorSyncRevision) return;
    const area: PhysicalRect = {
      position: { x: monitor.workArea.position.x, y: monitor.workArea.position.y },
      size: { width: monitor.workArea.size.width, height: monitor.workArea.size.height },
    };
    const key = monitorGeometryKey(area, monitor.scaleFactor);
    currentWorkArea = area;
    currentWorkAreaHeight = logicalWorkAreaHeight(area, monitor.scaleFactor);
    if (force || key !== currentMonitorKey) {
      currentMonitorKey = key;
      fitHeight();
      refreshHitRegionsAfterLayout();
    }
  } catch {
    // 모니터 조회 실패는 다음 이동·배율 이벤트에서 다시 시도한다.
  }
}

async function keepWindowInsideCurrentWorkArea() {
  // 이동이 끝나기 전에는 이전 모니터 작업영역으로 창을 끌어당기지 않는다.
  if (!currentWorkArea || monitorMoveTimer !== undefined) return;
  const [pos, size] = await Promise.all([appWindow.outerPosition(), appWindow.outerSize()]);
  const target = clampWindowToWorkArea(
    { x: pos.x, y: pos.y, width: size.width, height: size.height },
    currentWorkArea,
  );
  if (target.x !== pos.x || target.y !== pos.y) {
    await appWindow.setPosition(new PhysicalPosition(target.x, target.y));
  }
}

function fitHeight() {
  fitRevision += 1;
  if (fitQueued || fitRunning) return;
  fitQueued = true;
  // 비활성 WKWebView에서는 짧은 setTimeout도 크게 늦어질 수 있다. 같은 JS 작업에서
  // 몰린 요청만 microtask로 합치고, 실제 창 변경은 아래 단일 루프가 순서대로 맡는다.
  queueMicrotask(() => {
    fitQueued = false;
    void fitWindowToContent();
  });
}

async function fitWindowToContent() {
  if (fitRunning) return;
  fitRunning = true;
  let retryLatest = false;
  try {
    while (true) {
      const revision = fitRevision;
      const last = app.lastElementChild as HTMLElement | null;
      const bottomPad = parseFloat(getComputedStyle(app).paddingBottom) || 14;
      const content = last
        ? last.getBoundingClientRect().bottom -
          app.getBoundingClientRect().top +
          app.scrollTop +
          bottomPad
        : 40;
      const tbHeight = titlebarEl.offsetHeight;
      const total = tbHeight + content + 2; // 테두리
      const max = Math.floor((currentWorkAreaHeight ?? window.screen.availHeight) * 0.9);
      // 배율이 소수인 화면에서 round가 1px을 깎아 하단을 자르지 않게 올림한다.
      // 논리→물리 변환의 최근접 반올림까지 버티도록 콘텐츠보다 1px 여유를 둔다.
      const target = Math.ceil(Math.max(80, Math.min(total + 1, max)));
      // 컴팩트 모드는 창 자체도 좁게, 미니멀은 더 좁게 (150→120, 사용자 지시 —
      // 타이틀바 버튼은 한 줄을 포기하고 다음 줄로 흐른다)
      const width = viewMode === "minimal" ? 120 : viewMode === "compact" ? 240 : 360;
      // 크기 조절 기준은 "오른쪽 상단" — 목표 폭이 실제로 바뀌는 전환에서만
      // 우측 가장자리를 고정한다. (바깥 크기에는 그림자가 포함되므로 실측 폭과
      // 목표 폭을 비교하면 매번 어긋나 창이 조금씩 밀리는 버그가 있었다)
      const widthChanging = lastAppliedWidth !== 0 && lastAppliedWidth !== width;
      let rightEdge = 0;
      let topY = 0;
      if (widthChanging) {
        try {
          const [pos, size] = await Promise.all([
            appWindow.outerPosition(),
            appWindow.outerSize(),
          ]);
          rightEdge = pos.x + size.width;
          topY = pos.y;
        } catch {
          rightEdge = 0;
        }
      }
      // 위치를 읽는 동안 더 최신 내용·모드가 들어왔으면 낡은 크기는 적용하지 않는다.
      if (revision !== fitRevision) continue;
      try {
        await appWindow.setSize(new LogicalSize(width, target));
      } catch {
        // setSize 대기 중 더 최신 요청이 들어왔으면 실패한 옛 요청만 버리고 최신값은 재시도한다.
        retryLatest = revision !== fitRevision;
        break;
      }
      // setSize가 시작된 뒤 새 요청이 와도 이 트랜잭션의 실제 폭부터 기록한다.
      // 루프가 곧 최신 요청을 다시 적용하므로 오래된 크기가 최종값으로 남지 않는다.
      lastAppliedWidth = width;
      if (widthChanging && rightEdge !== 0) {
        try {
          // 새 폭의 실제 바깥 크기(그림자 포함)로 우측 가장자리를 되살린다
          const newSize = await appWindow.outerSize();
          await appWindow.setPosition(new PhysicalPosition(rightEdge - newSize.width, topY));
        } catch {
          // 위치 보정 실패는 치명적이지 않다
        }
      }
      try {
        await keepWindowInsideCurrentWorkArea();
      } catch {
        // 창이 이동 중이면 좌표 읽기·보정이 잠시 실패할 수 있다. 다음 이벤트가 재시도한다.
      }
      // 폭이 바뀌며 타이틀바가 줄바꿈(미니멀 두 줄)되면 계산에 쓴 높이가
      // 낡는다 — 실제 높이가 달라졌으면 새 높이로 다시 맞춘다 (타입3 하단
      // NET 행이 13px 잘리던 문제, 사용자 보고). 외부 요청이 들어왔거나 높이가
      // 달라졌으면 최신 DOM을 다시 재서 마지막 크기가 반드시 최신값이 되게 한다.
      // 히트 영역은 반드시 창 크기 변경이 "끝난 뒤" 보고해야 한다 —
      // 즉시 보고하면 옛 폭 기준 좌표가 남아 버튼 위 클릭이 투과돼 버린다
      refreshHitRegionsAfterLayout();
      if (revision !== fitRevision) continue;
      if (titlebarEl.offsetHeight !== tbHeight) {
        fitRevision += 1;
        continue;
      }
      break;
    }
  } finally {
    fitRunning = false;
    if (retryLatest && !fitQueued) {
      fitQueued = true;
      queueMicrotask(() => {
        fitQueued = false;
        void fitWindowToContent();
      });
    }
  }
}

// 크기 변경(모드 전환 등) 완료 시점의 백업 갱신 — 어떤 경로로 리사이즈되든 좌표를 다시 잡는다
void appWindow.onResized(() => {
  queueMicrotask(refreshHitRegionsAfterLayout);
});
void appWindow.onMoved(() => {
  // 드래그 도중 모니터 경계를 넘는 순간 창을 당겨 오지 않고, 이동이 끝난 뒤 맞춘다.
  scheduleMonitorSync();
});
void appWindow.onScaleChanged(() => scheduleMonitorSync(true));
void syncCurrentMonitor(true);

// 내용 높이가 바뀌는 지점(렌더 완료·사용량 로딩·고정 토글)에서 fitHeight()를 직접 부른다
// — app 요소 자체는 창 크기에 묶여 있어 관찰자로는 콘텐츠 변화를 못 잡는다
// 데모·스크린샷용 초기 모드 강제 (SWITCHER_VIEW 환경변수) —
// 진행 중인 첫 렌더가 있으면 render()가 큐잉해 단일 모드로 다시 그린다
void invoke<string | null>("initial_view_mode").then((mode) => {
  if (mode === "locked") mode = "compact"; // Type2 폐지(#41) — 옛 이름 이관
  if (mode === "normal" || mode === "compact" || mode === "minimal") {
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
  if (clamMode >= 0) applyClamshell(clamMode); // 클램셸 툴팁도 새 언어로
  document.getElementById("memobtn")!.setAttribute("title", t("memoTooltip"));
  document.getElementById("tfsdbtn")!.setAttribute("title", t("tfsdBtnTooltip"));
  document.getElementById("monbtn")!.setAttribute("title", t("monitorTooltip"));
  document.getElementById("privacybtn")!.setAttribute("title", t("privacyTooltip"));
  alphaSlider.title = t("alphaTooltip");
  lockBtn.title = t("typeTooltip");
}

// 블랙 모니터 — 모든 화면을 최상위 검은 막으로 (해제는 오버레이 쪽: 흔들기·ESC)
document.getElementById("blackbtn")!.addEventListener("click", () => {
  void invoke("black_on").catch((error) => toast(String(error), true));
});

// ── 클램셸 슬립 방지 (☕, macOS 전용) — off → 일회성 → 지속 → off ─────────
// 덮개를 닫아도 잠들지 않게 (터미널 AI 작업 유지). 켤 때만 관리자 암호 1회,
// 해제·복원(끄기·덮개 열림·종료·크래시)은 root 감시자가 암호 없이 처리한다.
const clamBtn = document.getElementById("clambtn") as HTMLButtonElement;
let clamMode = -1;
function applyClamshell(mode: number) {
  const wasHidden = clamBtn.hidden;
  clamMode = mode;
  clamBtn.hidden = mode < 0; // 미지원 플랫폼(Windows)에서는 버튼 자체가 없다
  clamBtn.classList.toggle("pinned", mode === 2);
  clamBtn.classList.toggle("half", mode === 1);
  clamBtn.title = mode === 1 ? t("clamOnce") : mode === 2 ? t("clamKeep") : t("clamOff");
  if (wasHidden !== clamBtn.hidden) fitHeight();
  refreshHitRegionsAfterLayout();
}
void invoke<number>("clamshell_mode").then(applyClamshell);
let clamBusy = false;
clamBtn.addEventListener("click", () => {
  if (clamBusy) return; // 관리자 승인 대기 중 연타 방지
  clamBusy = true;
  clamBtn.disabled = true; // 승인 창 대기·전환(최대 수초) 중임을 보이게 — 소리 없는 무시 방지
  invoke<number>("clamshell_cycle")
    .then(applyClamshell)
    .catch((error) => {
      toast(String(error), true);
      return invoke<number>("clamshell_mode").then(applyClamshell).catch(() => undefined);
    })
    .finally(() => {
      clamBusy = false;
      clamBtn.disabled = false;
    });
});
// 일회성 모드의 자동 해제(덮개 닫힘→열림)·잔존 복원이 끝나면 버튼 표시를 따라 맞춘다
void listen("clamshell-changed", () => {
  void invoke<number>("clamshell_mode").then(applyClamshell);
});
// 비활성 패널의 웹뷰는 위 이벤트 전달·반영이 밀릴 수 있다 (#52 계열, 실기기에서
// 복원 후 버튼이 켜짐으로 고착돼 보인 사례) — 커서가 위젯에 들어올 때마다 실상태를
// 당겨와 표시를 실제와 맞춘다. 상태 파일 읽기라 비용은 미미하다.
document.documentElement.addEventListener("mouseenter", () => {
  void invoke<number>("clamshell_mode").then(applyClamshell);
});

// 비활성 패널에서는 :active가 안 먹어 클릭 피드백이 없다 (실기기 사용자 보고)
// — 포인터 이벤트로 누름 상태(.pressing)를 직접 단다. 클릭은 확실히 도달하므로
// pointerdown도 도달한다.
document.querySelectorAll<HTMLButtonElement>(".tb-actions button").forEach((btn) => {
  btn.addEventListener("pointerdown", () => btn.classList.add("pressing"));
  const clearPressing = () => btn.classList.remove("pressing");
  btn.addEventListener("pointerup", clearPressing);
  btn.addEventListener("pointerleave", clearPressing);
  btn.addEventListener("pointercancel", clearPressing);
});

// 메모장 (Type2 전용 버튼) — 별도 창 토글. 내용·투명도는 메모창이 스스로 관리
document.getElementById("memobtn")!.addEventListener("click", () => {
  void invoke("memo_toggle").catch((error) => toast(String(error), true));
});

// 자율주행 (🚗) — 트레이 "TFSD 자동 전환"과 같은 플래그를 토글한다.
// 즉시 재렌더해 활성 카드의 T 배지·버튼 켜짐 표시가 바로 따라온다
document.getElementById("tfsdbtn")!.addEventListener("click", () => {
  invoke<boolean>("tfsd_toggle")
    .then((on) => {
      document.getElementById("tfsdbtn")!.classList.toggle("pinned", on);
      void render({ immediate: true });
    })
    .catch((error) => toast(String(error), true));
});

// ── 시스템 모니터 (📊) — 위젯 본체 안 SYSTEM 섹션 토글 ─────────────
// 별도 창은 메모장만이다 (사용자 지시) — 모니터는 위젯의 한 섹션으로 산다.
// 미니멀(Type3)은 사용량 전용 독트린(#41)에 따라 그리지 않는다.
interface SysStats {
  cpu: number;
  mem_used: number;
  mem_total: number;
  disk_read: number;
  disk_write: number;
  /// Windows: 물리 디스크 활성 시간 %(작업 관리자와 동일 지표), 맥: null
  disk_pct?: number | null;
  net_rx: number;
  net_tx: number;
}

const monBtn = document.getElementById("monbtn") as HTMLButtonElement;
let monitorOn = localStorage.getItem("switcher.monitor") === "1";
monBtn.classList.toggle("pinned", monitorOn);
monBtn.addEventListener("click", () => {
  monitorOn = !monitorOn;
  localStorage.setItem("switcher.monitor", monitorOn ? "1" : "0");
  monBtn.classList.toggle("pinned", monitorOn);
  void render({ immediate: true });
});

/// SYSTEM 섹션 골격 — 값은 monitorTick이 1초마다 id로 찾아 채운다
/// (재렌더로 노드가 갈려도 다음 틱이 새 노드를 채우므로 참조를 들고 있지 않는다)
function renderMonitor(target: DocumentFragment) {
  const section = document.createElement("section");
  section.className = "mon-section";
  const title = document.createElement("div");
  title.className = "section-title mon-title";
  const label = document.createElement("span");
  label.textContent = "SYSTEM";
  const beat = document.createElement("span");
  beat.id = "mon-beat";
  const mood = document.createElement("span");
  mood.id = "mon-mood";
  mood.textContent = "(・ᴗ・)";
  title.append(label, beat, mood);
  section.appendChild(title);
  for (const [key, name] of [
    ["cpu", "CPU"],
    ["mem", "MEM"],
    ["dsk", "DSK"],
    ["net", "NET"],
  ]) {
    const row = document.createElement("div");
    row.className = "mon-row";
    row.id = `mon-row-${key}`;
    const lab = document.createElement("span");
    lab.className = "mon-label";
    lab.textContent = name;
    // div여야 한다 — span(inline)은 width가 무시돼 채움 바가 안 그려진다
    // (사용량 카드의 .bar/.bar-fill과 동일하게, 사용자 보고로 수정)
    const bar = document.createElement("div");
    bar.className = "bar mon-bar";
    const fill = document.createElement("div");
    fill.className = `bar-fill mon-fill-${key}`;
    bar.appendChild(fill);
    const val = document.createElement("span");
    val.className = "mon-val";
    val.textContent = "--";
    row.append(lab, bar, val);
    section.appendChild(row);
    // CPU 줄 밑에 60초 스파크라인
    if (key === "cpu") {
      const spark = document.createElement("canvas");
      spark.id = "mon-spark";
      section.appendChild(spark);
    }
  }
  target.appendChild(section);
}

/// 정수 GB — 소수점·단위 문자 없음 (행 라벨이 무엇인지 말해주므로, 사용자 지시)
function gbInt(bytes: number): string {
  return String(Math.round(bytes / 1024 ** 3));
}

/// 정수 MB/s — 소수점·단위 문자 없음 (위와 동일). 유휴(1MB/s 미만)는 0으로
/// 나온다 — "논다"는 정보로 충분하고, 미세 활동은 게이지 바가 마저 보여준다
function mbsInt(bytesPerSec: number): string {
  return String(Math.round(bytesPerSec / 1024 ** 2));
}

const MON_HISTORY = 60;
const monHistory: number[] = [];
/// 네트워크 바의 기준 — 세션 최고 속도 (바닥 1MB/s: 유휴가 꽉 차 보이지 않게)
let monNetPeak = 1024 * 1024;
/// 디스크 바의 기준 — 세션 최고 I/O (바닥 32MB/s: 유휴 잔쓰기로 차 보이지 않게)
let monDskPeak = 32 * 1024 * 1024;
let monInflight = false;
let monLastTick = 0;

function drawMonSpark() {
  const spark = document.getElementById("mon-spark") as HTMLCanvasElement | null;
  if (!spark) return;
  const ctx = spark.getContext("2d");
  if (!ctx) return;
  const scale = window.devicePixelRatio || 1;
  const w = spark.clientWidth;
  const h = spark.clientHeight;
  // 재렌더로 캔버스가 새로 생겼거나 크기가 변한 경우에만 비트맵 재할당
  const pw = Math.round(w * scale);
  const ph = Math.round(h * scale);
  if (spark.width !== pw || spark.height !== ph) {
    spark.width = pw;
    spark.height = ph;
  }
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (monHistory.length < 2) return;
  const step = w / (MON_HISTORY - 1);
  const y = (v: number) => h - 1 - (v / 100) * (h - 2);
  ctx.beginPath();
  monHistory.forEach((v, i) => {
    const x = w - (monHistory.length - 1 - i) * step;
    if (i === 0) ctx.moveTo(x, y(v));
    else ctx.lineTo(x, y(v));
  });
  ctx.strokeStyle = "rgba(167, 139, 250, 0.9)";
  ctx.lineWidth = 1.2;
  ctx.stroke();
  ctx.lineTo(w, h);
  ctx.lineTo(w - (monHistory.length - 1) * step, h);
  ctx.closePath();
  ctx.fillStyle = "rgba(167, 139, 250, 0.15)";
  ctx.fill();
}

/// CPU 기분 — 한가하면 느긋, 바쁘면 진지, 뜨거우면 울상
function monMood(cpuPct: number): string {
  if (cpuPct < 40) return "(・ᴗ・)";
  if (cpuPct < 80) return "(•̀ᴗ•́)";
  return "(>﹏<)";
}

function monSetRow(key: string, percent: number, text: string) {
  const row = document.getElementById(`mon-row-${key}`);
  if (!row) return;
  const clamped = Math.max(0, Math.min(100, percent));
  const fill = row.querySelector<HTMLElement>(".bar-fill");
  if (fill) fill.style.width = `${clamped}%`;
  row.querySelector(".mon-bar")?.classList.toggle("hot", clamped >= 90);
  const val = row.querySelector<HTMLElement>(".mon-val");
  if (val) val.textContent = text;
}

/// 마지막 샘플 — 재렌더 직후 새 스켈레톤을 즉시 채우는 데 쓴다 (red-review:
/// 스무스 교체마다 SYSTEM 섹션만 '--'로 깜빡이던 문제)
let monLastStats: SysStats | null = null;

/// 샘플 하나를 화면에 그린다 — 상태(monHistory 등)는 건드리지 않는다
function paintMonitor(s: SysStats) {
  // 수치는 전부 정수·무단위 (사용자 지시: 라벨이 있으니 단위는 자명하다) —
  // CPU는 %, MEM은 GB, DSK·NET은 MB/s
  monSetRow("cpu", s.cpu, String(Math.round(s.cpu)));
  const mood = document.getElementById("mon-mood");
  if (mood) mood.textContent = monMood(s.cpu);
  monSetRow(
    "mem",
    (s.mem_used / Math.max(1, s.mem_total)) * 100,
    `${gbInt(s.mem_used)}/${gbInt(s.mem_total)}`,
  );
  // 디스크는 용량이 아니라 활동량(R/W 속도)이다 — 사용자 지시. 바는 Windows면
  // 물리 디스크 활성 시간 %(작업 관리자와 같은 지표 — "저긴 0%인데 여긴 차
  // 있다" 괴리 보고), 맥이면 세션 피크 대비 폴백
  const io = s.disk_read + s.disk_write;
  monSetRow(
    "dsk",
    s.disk_pct ?? (io / Math.max(1, monDskPeak)) * 100,
    `R${mbsInt(s.disk_read)} W${mbsInt(s.disk_write)}`,
  );
  const flow = s.net_rx + s.net_tx;
  monSetRow(
    "net",
    (flow / Math.max(1, monNetPeak)) * 100,
    `↓${mbsInt(s.net_rx)} ↑${mbsInt(s.net_tx)}`,
  );
}

async function monitorTick() {
  // 섹션이 화면에 없으면(꺼짐·재렌더 중) 조용히 건너뛴다.
  // document.hidden은 보지 않는다 — 별도 모니터 창 시절의 고아 조건으로, 맥은
  // 앱이 비활성이면(위젯의 평상시) 페이지가 hidden이라 SYSTEM이 얼어붙었다
  // (리뷰 #53). 샘플은 1초에 한 번짜리 경량 호출이다.
  if (!monitorOn || monInflight) return;
  if (!document.getElementById("mon-row-cpu")) return;
  monInflight = true;
  try {
    const s = await invoke<SysStats>("stats_read");
    // 응답을 기다리는 사이 📊가 꺼졌으면 폐기 — 꺼진 동안의 샘플이
    // monHistory·monLastTick을 오염시키지 않게 (red-review)
    if (!monitorOn) return;
    // 오래 쉬었다 돌아왔으면 스파크라인을 새로 시작 — 공백 전후가
    // 연속 60초처럼 이어져 그려지는 왜곡 방지 (red-review)
    const now = Date.now();
    if (monLastTick > 0 && now - monLastTick > 5000) monHistory.length = 0;
    monLastTick = now;
    monNetPeak = Math.max(monNetPeak, s.net_rx + s.net_tx);
    monDskPeak = Math.max(monDskPeak, s.disk_read + s.disk_write);
    monLastStats = s;
    monHistory.push(s.cpu);
    if (monHistory.length > MON_HISTORY) monHistory.shift();
    paintMonitor(s);
    drawMonSpark();
    // 심장박동 — 새 샘플이 도착했다는 신호 (재렌더 후 재채움에는 안 뛴다)
    const beat = document.getElementById("mon-beat");
    if (beat) {
      beat.classList.add("pump");
      window.setTimeout(() => beat?.classList.remove("pump"), 180);
    }
  } catch {
    // 일시 실패는 다음 틱이 만회한다
  } finally {
    monInflight = false;
  }
}
window.setInterval(() => void monitorTick(), 1000);

// 데모·검증용 (SWITCHER_OPEN=monitor): SYSTEM 섹션을 켠 채 시작 — 저장하지 않는
// 일회성 표시라 localStorage는 건드리지 않는다
void invoke<string>("initial_open").then((open) => {
  if (open.includes("monitor") && !monitorOn) {
    monitorOn = true;
    monBtn.classList.add("pinned");
    void render({ immediate: true });
  }
});

// ── 섹션 순서 (Type1 드래그 앤 드랍) ─────────────────────────────
// 머리글을 잡아 다른 섹션 위/아래에 놓으면 순서가 바뀌고 저장된다.
// 순서는 모든 타입에 적용되지만, 바꾸는 조작은 Type1에서만 (위젯 모드는 투과·표시 전용)
type SectionKey = ProviderId | "github" | "display" | "system";
const SECTION_ORDER_DEFAULT: SectionKey[] = ["claude", "codex", "github", "display", "system"];

let sectionOrder: SectionKey[] = (() => {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem("switcher.order") ?? "[]");
    if (!Array.isArray(raw)) return [...SECTION_ORDER_DEFAULT];
    // 알 수 없는 키·중복 제거, 빠진 키는 기본 순서대로 뒤에 보충 —
    // 새 섹션이 추가된 업데이트 후에도 저장된 순서가 안전하게 살아난다
    const valid = raw.filter(
      (k, i): k is SectionKey =>
        SECTION_ORDER_DEFAULT.includes(k as SectionKey) && raw.indexOf(k) === i,
    );
    const missing = SECTION_ORDER_DEFAULT.filter((k) => !valid.includes(k));
    return [...valid, ...missing];
  } catch {
    return [...SECTION_ORDER_DEFAULT];
  }
})();

let dragKey: SectionKey | null = null;
/// dragstart 시각 — 시작 프레임에 코얼레싱돼 남은 pointermove가 래치를 바로
/// 풀어버리지 않게 하는 가드 기준
let dragStartAt = 0;

function clearDropMarks() {
  document
    .querySelectorAll(".drop-before, .drop-after")
    .forEach((el) => el.classList.remove("drop-before", "drop-after"));
}

/// 드래그 래치 해제 — dragend와 유실 복구(아래 pointermove)가 공유한다
function clearDragState() {
  dragKey = null;
  clearDropMarks();
  document.querySelectorAll(".dragging").forEach((el) => el.classList.remove("dragging"));
}

// dragend 유실 복구 (리뷰 #53: 고착된 dragKey가 렌더 스왑을 영구 차단하던 문제):
// HTML5 드래그 중에는 pointermove가 발화하지 않는다(드래그 이벤트로 대체) —
// 래치가 있는데 pointermove가 왔다는 것은 dragend가 유실된 채 드래그가 끝났다는
// 뜻이므로 즉시 푼다. 시간 TTL이 아니라서 창 밖에 오래 머무는 살아있는 드래그를
// 오살하지 않는다 (red-review — 2초 TTL의 조기 해제·파일 드래그 연장 맞바꿈).
// 200ms 가드: dragstart 발화 프레임의 잔여 pointermove 한 방에 시작 직후
// 풀리는 것 방지 — 드래그 중엔 pointermove가 없어 이 가드는 오살과 무관하다.
window.addEventListener("pointermove", () => {
  if (dragKey !== null && Date.now() - dragStartAt > 200) clearDragState();
});

// OS 파일 드롭 무해화 (리뷰 #53): dragDropEnabled:false로 네이티브 가로채기를
// 껐으므로(섹션 드래그용) 기본 동작 차단은 우리 몫 — 안 막으면 웹뷰가 드롭된
// 파일로 내비게이션해 위젯이 통째로 죽는다. 내부 섹션 드래그와의 구분은 래치가
// 아니라 Files 타입으로 — 래치가 고착돼도 차단이 뚫리지 않는다 (red-review)
window.addEventListener("dragover", (event) => {
  if (!event.dataTransfer?.types.includes("Files")) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "none";
});
// 내부 섹션 드롭은 섹션 핸들러가 이미 처리했다 — 기본 동작(드롭된 파일로
// 내비게이션)만 무조건 죽인다
window.addEventListener("drop", (event) => {
  event.preventDefault();
});

function moveSection(from: SectionKey, to: SectionKey, before: boolean) {
  const rest = sectionOrder.filter((k) => k !== from);
  const idx = rest.indexOf(to);
  if (idx < 0) return;
  rest.splice(before ? idx : idx + 1, 0, from);
  sectionOrder = rest;
  localStorage.setItem("switcher.order", JSON.stringify(sectionOrder));
  void render({ immediate: true });
}

/// 머리글에 드래그 시작을 붙인다 — 접기(collapseSectionsInPlace)가 머리글을
/// 새로 만들 때도 다시 불러야 한다 (red-review: 접힌 뒤 드래그 시작 불가)
function attachHeaderDrag(section: HTMLElement, key: SectionKey) {
  // .compact-head는 안 찾는다 — 호출부가 전부 normal 모드 게이트라 컴팩트
  // 머리글에는 닿을 일이 없다 (리뷰 #53: 죽은 셀렉터 정리)
  const header = section.querySelector<HTMLElement>(".section-title, .section-toggle");
  if (!header) return;
  header.draggable = true;
  header.addEventListener("dragstart", (event) => {
    dragKey = key;
    dragStartAt = Date.now();
    // WebView2에서 데이터가 비어 있으면 드래그가 시작되지 않는 경우가 있다
    event.dataTransfer?.setData("text/plain", key);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    event.dataTransfer?.setDragImage(section, 24, 12);
    // 흐림은 고스트 캡처 다음 프레임에 — 같은 틱에 걸면 고스트까지 반투명해진다
    // (Chromium 계열 공통, red-review)
    requestAnimationFrame(() => {
      if (dragKey === key) section.classList.add("dragging");
    });
  });
  header.addEventListener("dragend", () => clearDragState());
}

/// 섹션 하나에 드래그 이동을 붙인다 — 드래그 시작은 머리글에서만
/// (본문에는 슬라이더·입력칸·버튼이 있어 섹션 전체를 draggable로 만들면 오동작)
function enableSectionDrag(section: HTMLElement, key: SectionKey, mode: ViewMode) {
  if (mode !== "normal") return;
  attachHeaderDrag(section, key);
  section.addEventListener("dragover", (event) => {
    if (!dragKey || dragKey === key) return;
    event.preventDefault(); // 드랍 허용
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    clearDropMarks();
    section.classList.add(dropBefore(section, event.clientY) ? "drop-before" : "drop-after");
  });
  section.addEventListener("dragleave", (event) => {
    // 자식(카드·바) 위로 지나갈 때도 dragleave가 연발한다 — 진짜로 섹션을
    // 벗어날 때만 표시를 지운다 (red-review: 표시선 깜빡임)
    if (section.contains(event.relatedTarget as Node | null)) return;
    section.classList.remove("drop-before", "drop-after");
  });
  section.addEventListener("drop", (event) => {
    if (!dragKey || dragKey === key) return;
    event.preventDefault();
    const from = dragKey;
    dragKey = null;
    clearDropMarks();
    moveSection(from, key, dropBefore(section, event.clientY));
  });
}

/// 드랍 위치 판정 — 접힌 섹션(머리글 한 줄, ~28px)은 midpoint 폭이 좁아
/// "뒤에 놓기"가 사실상 불가능했다 (red-review). 커서가 아래 40% 밴드에
/// 들어오면 뒤로 판정해 마지막 자리에도 자연스럽게 놓인다
function dropBefore(section: HTMLElement, clientY: number): boolean {
  const rect = section.getBoundingClientRect();
  return clientY < rect.top + rect.height * 0.6;
}

// 이메일 가리기 (🙈) — 표시만 블러 처리, 동작·데이터는 그대로. 재시작 후에도 유지
const privacyBtn = document.getElementById("privacybtn") as HTMLButtonElement;
function applyPrivacy(on: boolean) {
  document.body.classList.toggle("privacy", on);
  privacyBtn.classList.toggle("pinned", on);
}
let privacyOn = localStorage.getItem("switcher.privacy") === "1";
applyPrivacy(privacyOn);
privacyBtn.addEventListener("click", () => {
  privacyOn = !privacyOn;
  localStorage.setItem("switcher.privacy", privacyOn ? "1" : "0");
  applyPrivacy(privacyOn);
});

// 수동 전환 감지 → TFSD 해제 알림 (운전대를 잡으면 자율주행이 풀린다)
void listen("tfsd-disengaged", () => {
  toast(t("tfsdDisengaged"));
});

// TFSD 자동 전환 알림 — 백그라운드에서 계정이 바뀌었으니 다시 그린다
// (로그인 패널이 열려 있으면 재렌더를 미룬다 — 세션 보호 정책과 동일)
void listen<{ provider: string; from: string; to: string }>("tfsd-switched", (event) => {
  toast(t("tfsdSwitched", event.payload));
  if (!loginOpen) void render({ immediate: true });
});

// 실행 시 자동 업데이트 결과 — 교체는 이미 끝났고 다음 실행부터 새 버전이다
void listen<string>("update-ready", (event) => {
  toast(t("updateReady", { ver: event.payload }));
});

// 수동 "업데이트 확인"이 새 버전을 적용했다 — 잠깐 알리고 러스트가 재시작한다
void listen<string>("update-restarting", (event) => {
  toast(t("updateRestarting", { ver: event.payload }));
});

// 트레이(설정 → 표시 기능)에서 체크가 바뀌면 다시 그린다
void listen("visibility-changed", () => {
  void loadVisibility().then(() => render({ immediate: true }));
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

// 시작: 저장된 언어·표시 기능을 먼저 읽고 첫 렌더 — 그렸다 갈아엎는 깜빡임을 피한다
void (async () => {
  try {
    const saved = await invoke<string>("get_language");
    // 조회 응답을 기다리는 사이 트레이 이벤트로 언어가 이미 정해졌으면 구값으로 덮지 않는다
    if (!langFromEvent) setLang(saved);
  } catch {
    // 설정을 못 읽으면 한국어로 진행
  }
  await loadVisibility();
  applyStaticText();
  void render();
})();
