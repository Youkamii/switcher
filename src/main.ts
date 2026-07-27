import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type ProfileInfo = {
  name: string;
  id: string;
  email: string | null;
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

type Usage = { windows: UsageWindow[] };

const PROVIDERS = [
  { id: "claude", title: "클로드" },
  { id: "codex", title: "코덱스" },
] as const;

type ProviderId = (typeof PROVIDERS)[number]["id"];

const app = document.getElementById("app")!;
const toastEl = document.getElementById("toast")!;
let toastTimer: number | undefined;

function toast(message: string, isError = false) {
  toastEl.textContent = message;
  toastEl.classList.toggle("error", isError);
  toastEl.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (toastEl.hidden = true), 3500);
}

function formatReset(resetsAt: string | null): string {
  if (!resetsAt) return "";
  const ts = /^\d+$/.test(resetsAt) ? Number(resetsAt) * 1000 : Date.parse(resetsAt);
  if (Number.isNaN(ts)) return "";
  const diff = ts - Date.now();
  if (diff <= 0) return "리셋됨";
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (days >= 1) return `리셋까지 ${days}일 ${hours % 24}시간`;
  if (hours >= 1) return `리셋까지 ${hours}시간 ${minutes % 60}분`;
  return `리셋까지 ${minutes}분`;
}

function usageRow(win: UsageWindow): HTMLElement {
  const row = document.createElement("div");
  row.className = "usage-row";

  const label = document.createElement("span");
  label.className = "usage-label";
  label.textContent = win.label;
  label.title = `${win.label} ${formatReset(win.resets_at)}`.trim();

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

  row.append(label, bar, pct);
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
    const primary = usage.windows[0];
    const note = formatReset(primary.resets_at);
    if (note) {
      const noteEl = document.createElement("div");
      noteEl.className = "usage-note";
      noteEl.textContent = note;
      box.appendChild(noteEl);
    }
  } catch (error) {
    box.textContent = "";
    const err = document.createElement("div");
    err.className = "usage-error";
    err.textContent = String(error);
    box.appendChild(err);
  }
}

function profileCard(provider: ProviderId, profile: ProfileInfo): HTMLElement {
  const card = document.createElement("div");
  card.className = "card" + (profile.active ? " active" : "");

  const head = document.createElement("div");
  head.className = "card-head";
  const name = document.createElement("span");
  name.className = "card-name";
  name.textContent = profile.name;
  const email = document.createElement("span");
  email.className = "card-email";
  email.textContent = profile.email ?? "";
  head.append(name, email);
  if (profile.active) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = "활성";
    head.appendChild(badge);
  }
  card.appendChild(head);

  // 활성 프로필은 활성 파일(항상 최신 토큰), 비활성은 보관함 토큰으로 조회
  void loadUsage(provider, card, profile.active ? null : profile.name);

  const actions = document.createElement("div");
  actions.className = "card-actions";
  if (!profile.active) {
    const switchBtn = document.createElement("button");
    switchBtn.className = "primary";
    switchBtn.textContent = "이 계정으로 전환";
    switchBtn.addEventListener("click", async () => {
      switchBtn.disabled = true;
      try {
        await invoke("switch_profile", { provider, name: profile.name });
        toast(`전환 완료 — 새로 여는 터미널부터 '${profile.name}' 계정이 적용됩니다`);
        await render();
      } catch (error) {
        toast(String(error), true);
        switchBtn.disabled = false;
      }
    });
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

function addAccountHelp(provider: ProviderId, section: HTMLElement) {
  const toggle = document.createElement("button");
  toggle.className = "link";
  toggle.textContent = "다른 계정은 어떻게 추가하나요?";
  const help = document.createElement("div");
  help.className = "help";
  help.hidden = true;
  const loginCmd = provider === "claude" ? "claude 실행 후 /login" : "codex login";
  help.innerHTML = "";
  const lines = [
    "① 지금 계정을 아래 입력칸으로 먼저 저장",
    `② 터미널에서 <code>${loginCmd}</code> 으로 다른 계정 로그인`,
    "③ 여기 다시 와서 새 이름으로 저장",
    "이후에는 버튼 한 번으로 전환 — 재로그인이 필요 없습니다",
  ];
  for (const line of lines) {
    const p = document.createElement("div");
    p.innerHTML = line;
    help.appendChild(p);
  }
  toggle.addEventListener("click", () => (help.hidden = !help.hidden));
  section.append(toggle, help);
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
      hint.textContent = "로그인 정보가 없습니다 — 먼저 CLI에서 로그인하세요";
      section.appendChild(hint);
      app.appendChild(section);
      return;
    }

    if (snap.live && !snap.live_saved) {
      const hint = document.createElement("p");
      hint.className = "hint warn";
      hint.textContent = `현재 로그인 계정(${snap.live.email ?? snap.live.id})이 아직 프로필로 저장되지 않았습니다`;
      section.appendChild(hint);
    }

    for (const profile of snap.profiles) {
      section.appendChild(profileCard(provider, profile));
    }

    saveForm(provider, section);
    addAccountHelp(provider, section);
  } catch (error) {
    const err = document.createElement("p");
    err.className = "usage-error";
    err.textContent = String(error);
    section.appendChild(err);
  }

  app.appendChild(section);
}

let rendering = false;

async function render() {
  // 새로고침 연타·자동 주기와의 경합으로 화면이 겹쳐 그려지는 것을 막는다
  if (rendering) return;
  rendering = true;
  try {
    app.textContent = "";
    for (const { id, title } of PROVIDERS) {
      await renderProvider(id, title);
    }
  } finally {
    rendering = false;
  }
}

/// 자동 새로고침이 입력 중인 프로필 이름을 날리지 않게 한다
function userIsTyping(): boolean {
  const el = document.activeElement;
  return el instanceof HTMLInputElement && el.value.trim().length > 0;
}

const appWindow = getCurrentWindow();
// 위젯은 기본이 "항상 위" — 처음 실행(저장값 없음)이면 켠 상태로 시작
const storedPin = localStorage.getItem("switcher.pinned");
let pinned = storedPin === null ? true : storedPin === "1";

async function applyPin(button: HTMLButtonElement) {
  await appWindow.setAlwaysOnTop(pinned);
  button.classList.toggle("pinned", pinned);
  button.textContent = pinned ? "고정됨" : "고정";
}

const pinBtn = document.getElementById("pin") as HTMLButtonElement;
pinBtn.addEventListener("click", () => {
  pinned = !pinned;
  localStorage.setItem("switcher.pinned", pinned ? "1" : "0");
  void applyPin(pinBtn);
});
void applyPin(pinBtn);

document.getElementById("hide")!.addEventListener("click", () => void appWindow.hide());
document.getElementById("refresh")!.addEventListener("click", () => void render());
window.setInterval(() => {
  if (!userIsTyping()) void render();
}, 5 * 60 * 1000);
void render();
