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

function loginPanel(
  prompt: LoginPrompt,
  onDone: () => void,
  onCancel: () => void,
): HTMLElement {
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

  const actions = document.createElement("div");
  actions.className = "add-row";

  if (prompt.needs_code) {
    const input = document.createElement("input");
    input.placeholder = "로그인 후 받은 코드 붙여넣기";
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
        onDone();
      } catch (error) {
        toast(String(error), true);
        okBtn.disabled = false;
        input.disabled = false;
        okBtn.textContent = "확인";
      }
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
        onDone();
      } catch (error) {
        toast(String(error), true);
        onCancel();
      }
    })();
  }

  const cancelBtn = document.createElement("button");
  cancelBtn.className = "link";
  cancelBtn.textContent = "취소";
  cancelBtn.addEventListener("click", () => {
    void invoke("cancel_login");
    onCancel();
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
    addBtn.disabled = true;
    addBtn.textContent = "로그인 주소 받는 중…";
    try {
      const prompt = await invoke<LoginPrompt>("start_login", { provider });
      addBtn.hidden = true;
      loginOpen = true;
      slot.appendChild(
        loginPanel(
          prompt,
          () => {
            loginOpen = false;
            void render();
          },
          () => {
            loginOpen = false;
            slot.textContent = "";
            addBtn.hidden = false;
            addBtn.disabled = false;
            addBtn.textContent = "＋ 계정 추가";
          },
        ),
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
function userIsBusy(): boolean {
  const el = document.activeElement;
  const typing = el instanceof HTMLInputElement && el.value.trim().length > 0;
  return typing || loginOpen;
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
