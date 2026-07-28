import { invoke } from "@tauri-apps/api/core";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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

type Usage = { windows: UsageWindow[]; stale?: boolean };

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
  reset.title = "리셋까지 남은 시간";

  row.append(label, bar, pct, reset);
  return row;
}

async function loadUsage(provider: ProviderId, card: HTMLElement, profile: string | null) {
  const box = document.createElement("div");
  box.className = "usage-box";
  const loading = document.createElement("div");
  loading.className = "usage-note";
  loading.textContent = "사용량 불러오는 중…";
  box.appendChild(loading);
  card.appendChild(box);

  try {
    const usage = await invoke<Usage>("fetch_usage", { provider, profile });
    box.textContent = "";
    if (usage.windows.length === 0) {
      const empty = document.createElement("div");
      empty.className = "usage-note";
      empty.textContent = "표시할 사용량 정보가 없습니다";
      box.appendChild(empty);
      return;
    }
    for (const win of usage.windows) box.appendChild(usageRow(win));
    if (usage.stale) {
      // 갱신이 잠시 막힌 상태 — 기존 수치를 살짝 흐리게 두고 위에 작게 알린다
      box.classList.add("stale");
      const overlay = document.createElement("div");
      overlay.className = "stale-overlay";
      overlay.textContent = "사용량 조회 대기중";
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

function profileCard(provider: ProviderId, profile: ProfileInfo): HTMLElement {
  const card = document.createElement("div");
  card.className = "card" + (profile.active ? " active" : "");

  const head = document.createElement("div");
  head.className = "card-head";
  // 사용자는 이메일로 계정을 구분한다 — 프로필 이름은 안 보여주고 이메일만
  const email = document.createElement("span");
  email.className = "card-name";
  email.textContent = profile.email ?? profile.name;
  email.title = `프로필 이름: ${profile.name}`;
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

  // 활성 프로필은 활성 파일(항상 최신 토큰), 비활성은 보관함 토큰으로 조회
  void loadUsage(provider, card, profile.active ? null : profile.name);

  const who = profile.email ?? profile.name;
  let switching = false;
  const doSwitch = async (disable?: HTMLButtonElement) => {
    if (switching) return;
    switching = true;
    if (disable) disable.disabled = true;
    try {
      await invoke("switch_profile", { provider, name: profile.name });
      toast(`전환 완료 — 새로 여는 터미널부터 ${who} 계정이 적용됩니다`);
      await render();
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
    switchBtn.textContent = "이 계정으로 전환";
    switchBtn.addEventListener("click", () => void doSwitch(switchBtn));
    actions.appendChild(switchBtn);
  }

  const deleteBtn = document.createElement("button");
  deleteBtn.textContent = "삭제";
  let armed = false;
  deleteBtn.addEventListener("click", async () => {
    if (!armed) {
      armed = true;
      deleteBtn.textContent = "정말 삭제할까요?";
      deleteBtn.classList.add("danger-armed");
      window.setTimeout(() => {
        armed = false;
        deleteBtn.textContent = "삭제";
        deleteBtn.classList.remove("danger-armed");
      }, 3000);
      return;
    }
    deleteBtn.disabled = true;
    try {
      await invoke("delete_profile", { provider, name: profile.name });
      toast(`'${profile.name}' 프로필을 삭제했습니다 (로그인 자체는 유지됩니다)`);
      await render();
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
    button.textContent = "복사됨";
    window.setTimeout(() => (button.textContent = label), 1500);
  } catch {
    toast("복사에 실패했습니다 — 직접 선택해 복사하세요", true);
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
  btn.textContent = "복사";
  btn.addEventListener("click", () => void copyText(value, btn, "복사"));
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
  steps.textContent = prompt.needs_code
    ? "① 아래 주소를 원하는 브라우저에 붙여넣어 로그인 ② 화면에 뜨는 코드를 복사해 아래에 붙여넣기"
    : "① 아래 주소를 원하는 브라우저에 붙여넣기 ② 그 화면에 아래 코드를 입력하면 자동으로 완료됩니다";
  panel.appendChild(steps);

  panel.appendChild(copyBox("로그인 주소", prompt.url, false));
  if (prompt.device_code) {
    panel.appendChild(copyBox("일회용 코드 (15분 유효)", prompt.device_code, true));
  }

  if (prompt.needs_code) {
    const actions = document.createElement("div");
    actions.className = "add-row";
    const input = document.createElement("input");
    input.placeholder = "로그인 후 받은 코드 붙여넣기";
    input.maxLength = 64;
    const okBtn = document.createElement("button");
    okBtn.className = "primary";
    okBtn.textContent = "확인";
    const submit = async () => {
      const code = input.value.trim();
      if (!code) {
        toast("코드를 붙여넣으세요", true);
        return;
      }
      okBtn.disabled = true;
      input.disabled = true;
      okBtn.textContent = "확인 중…";
      try {
        const result = await invoke<LoginOutcome>("submit_login_code", { code });
        reportLogin(result);
      } catch (error) {
        // 코드 제출 후의 실패는 세션이 이미 끝난 상태라 재시도가 불가능하다 —
        // 패널을 닫고 처음부터 다시 시작하게 안내한다
        toast(`${String(error)} — '계정 추가'로 처음부터 다시 시도하세요`, true);
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
    waiting.textContent = "브라우저에서 코드 입력을 기다리는 중…";
    panel.appendChild(waiting);
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
  cancelBtn.textContent = "취소";
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
      ? `'${result.profile}' 계정(${who}) 로그인을 갱신했습니다`
      : `계정 추가 완료 — '${result.profile}' (${who})`,
  );
}

function addAccountButton(provider: ProviderId, section: HTMLElement) {
  const row = document.createElement("div");
  row.className = "add-row";

  const addBtn = document.createElement("button");
  addBtn.className = "primary";
  addBtn.textContent = "＋ 계정 추가";
  row.appendChild(addBtn);
  section.appendChild(row);

  const slot = document.createElement("div");
  section.appendChild(slot);

  addBtn.addEventListener("click", async () => {
    if (loginOpen) {
      toast("이미 로그인이 진행 중입니다 — 진행 중인 패널을 먼저 끝내세요", true);
      return;
    }
    addBtn.disabled = true;
    addBtn.textContent = "로그인 주소 받는 중…";
    try {
      const prompt = await invoke<LoginPrompt>("start_login", { provider });
      addBtn.hidden = true;
      loginOpen = true;
      slot.appendChild(
        loginPanel(prompt, () => {
          loginOpen = false;
          void render();
        }),
      );
    } catch (error) {
      toast(String(error), true);
      addBtn.disabled = false;
      addBtn.textContent = "＋ 계정 추가";
    }
  });
}

function saveForm(provider: ProviderId, section: HTMLElement) {
  const row = document.createElement("div");
  row.className = "save-row";
  const input = document.createElement("input");
  input.placeholder = "프로필 이름 (영문·숫자·-·_)";
  input.maxLength = 32;
  const saveBtn = document.createElement("button");
  saveBtn.textContent = "현재 계정 저장";
  const submit = async () => {
    const name = input.value.trim();
    if (!name) {
      toast("프로필 이름을 입력하세요", true);
      return;
    }
    try {
      await invoke("save_profile", { provider, name });
      toast(`현재 계정을 '${name}' 프로필로 저장했습니다`);
      await render();
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

async function renderProvider(provider: ProviderId, title: string) {
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
      hint.textContent = "저장된 계정이 없습니다 — 아래 버튼으로 추가하세요";
      section.appendChild(hint);
      addAccountButton(provider, section);
      app.appendChild(section);
      return;
    }

    if (snap.live && !snap.live_saved) {
      const hint = document.createElement("p");
      hint.className = "hint warn";
      hint.textContent = `현재 로그인 계정(${snap.live.email ?? snap.live.id})이 아직 프로필로 저장되지 않았습니다 — 아래 입력칸으로 저장하세요`;
      section.appendChild(hint);
    }

    for (const profile of snap.profiles) {
      section.appendChild(profileCard(provider, profile));
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

  app.appendChild(section);
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

/// 컴팩트용 리셋 시간: 24시간 이상이면 일/시(5/17), 그 밑이면 시:분(2:21)
function compactReset(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const ts = /^\d+$/.test(resetsAt) ? Number(resetsAt) * 1000 : Date.parse(resetsAt);
  if (Number.isNaN(ts)) return "";
  const diff = ts - Date.now();
  if (diff <= 0) return "0:00";
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (hours >= 24) return `${days}/${String(hours % 24).padStart(2, "0")}`;
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
  email.title = `프로필 이름: ${profile.name}`;
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
      reset.title = "리셋까지 남은 시간";
      row.append(label, bar, reset);
      card.appendChild(row);
    }
  } catch {
    // 컴팩트 모드에서는 조회 실패를 조용히 넘긴다 — 다음 주기에 다시 시도
  }
  return card;
}

/// 컴팩트 모드: Type 2의 축소판 — 모든 계정이 나오고 더블클릭 전환도 된다
async function renderProviderCompact(provider: ProviderId, title: string) {
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
    app.appendChild(section);

    for (const profile of snap.profiles) {
      section.appendChild(await compactCard(provider, profile));
    }
  } catch {
    // 목록 실패도 조용히 — 컴팩트는 표시 전용
  }
}

let renderQueued = false;

async function render() {
  // 새로고침 연타·자동 주기와의 경합으로 화면이 겹쳐 그려지는 것을 막고,
  // 그리는 도중 재요청이 오면 끝난 뒤 한 번 더 그린다
  if (rendering) {
    renderQueued = true;
    return;
  }
  rendering = true;
  try {
    do {
      renderQueued = false;
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
      app.textContent = "";
      for (const { id, title } of PROVIDERS) {
        if (mode === "compact") {
          await renderProviderCompact(id, title);
        } else {
          await renderProvider(id, title);
        }
      }
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
      // 전환된 카드가 살짝 빛나고 나서 다시 그린다
      el?.classList.add("switch-flash");
      if (!demoMode) {
        const who = el?.querySelector(".card-name")?.textContent ?? event.payload.name;
        toast(`전환 완료 — 새로 여는 터미널부터 ${who} 계정이 적용됩니다`);
      }
      window.setTimeout(() => void render(), 380);
    } else if (!demoMode) {
      toast(event.payload.error ?? "전환 실패", true);
    }
  },
);

lockBtn.addEventListener("click", () => {
  viewMode = viewMode === "normal" ? "locked" : viewMode === "locked" ? "compact" : "normal";
  localStorage.setItem("switcher.viewmode", viewMode);
  applyViewMode();
  // 모드가 바뀌면 화면 구성이 달라진다 — 다시 그린다 (열려 있던 로그인 패널도 정리됨)
  void render();
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
    void render();
  }
});

document.getElementById("refresh")!.addEventListener("click", () => {
  if (loginOpen) {
    toast("로그인을 진행 중입니다 — 끝내거나 취소한 뒤 새로고침하세요", true);
    return;
  }
  void render();
});
window.setInterval(() => {
  if (!userIsBusy()) void render();
}, 5 * 60 * 1000);
void render();
