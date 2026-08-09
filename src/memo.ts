/// 메모창 — Type2 위젯의 부속 메모장 (탭 1~5, 투명도 독립).
///
/// - 내용·활성 탭·투명도는 통째로 `~/.switcher/memo.json`에 저장된다 (Rust memo_save).
/// - 저장은 입력 후 600ms 디바운스, 탭 전환·창 숨김·포커스 이탈 시엔 즉시 플러시 —
///   숨겨진 채 앱이 꺼져도 마지막 입력이 남는다.
/// - 닫기(✕·ESC)는 hide만 한다 — 창은 재사용되고 다음 토글에 즉시 뜬다.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setLang, t } from "./i18n";

interface MemoData {
  tabs: string[];
  active: number;
  alpha: number;
}

const memoWindow = getCurrentWindow();
const textEl = document.getElementById("mtext") as HTMLTextAreaElement;
const alphaEl = document.getElementById("malpha") as HTMLInputElement;
const tabEls = [...document.querySelectorAll<HTMLButtonElement>(".tab")];

let data: MemoData = { tabs: ["", "", "", "", ""], active: 0, alpha: 100 };
/// memo_load가 끝나기 전의 flush(블러·ESC 등)가 기본값(빈 탭)으로 저장 파일을
/// 덮어쓰는 사고 방지 (red-review) — 로드 완료 전에는 어떤 저장도 하지 않는다
let loaded = false;

function applyAlpha(percent: number) {
  const clamped = Math.max(0, Math.min(100, percent));
  document.documentElement.style.setProperty("--bg-alpha", (clamped / 100).toFixed(3));
  alphaEl.value = String(clamped);
}

function renderTabs() {
  tabEls.forEach((el, i) => {
    el.classList.toggle("active", i === data.active);
    el.classList.toggle("filled", data.tabs[i].trim().length > 0);
  });
}

/// 화면 → 데이터 동기화 (현재 탭 본문만)
function pull() {
  data.tabs[data.active] = textEl.value;
}

let saveTimer = 0;
function scheduleSave() {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void flush(), 600);
}

async function flush() {
  if (!loaded) return;
  window.clearTimeout(saveTimer);
  pull();
  try {
    await invoke("memo_save", { data });
  } catch {
    // 저장 실패(디스크 등)는 다음 입력·플러시가 재시도한다 — 메모창은 계속 동작
  }
}

textEl.addEventListener("input", () => {
  pull();
  renderTabs();
  scheduleSave();
});

for (const el of tabEls) {
  el.addEventListener("click", () => {
    const i = Number(el.dataset.i);
    if (i === data.active) return;
    pull();
    data.active = i;
    textEl.value = data.tabs[i];
    renderTabs();
    void flush();
    textEl.focus();
  });
}

alphaEl.addEventListener("input", () => {
  data.alpha = Number(alphaEl.value);
  applyAlpha(data.alpha);
  scheduleSave();
});

/// 닫기 = 숨기기 — 창을 파괴하지 않아 다음 토글이 즉시다
async function hideSelf() {
  await flush();
  await memoWindow.hide();
}

document.getElementById("mclose")!.addEventListener("click", () => void hideSelf());
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") void hideSelf();
});

// 포커스가 떠날 때·숨겨질 때 즉시 저장 — 디바운스 대기 중 유실 방지
window.addEventListener("blur", () => void flush());
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") void flush();
});

// 테두리 없는 창의 리사이즈 — 우하단 그립을 잡으면 창 크기 조절
document.getElementById("grip")!.addEventListener("mousedown", (event) => {
  event.preventDefault();
  void memoWindow.startResizeDragging("SouthEast");
});

function applyText() {
  textEl.placeholder = t("memoPlaceholder");
  document.getElementById("mclose")!.setAttribute("title", t("memoClose"));
  document.getElementById("mdrag")!.setAttribute("title", t("dragHandle"));
  alphaEl.title = t("memoAlphaTooltip");
}

// 초기화: 언어 → 저장된 메모 순서로. 언어 변경(트레이)도 실시간 반영
void (async () => {
  try {
    setLang(await invoke<string>("get_language"));
  } catch {
    // 언어를 못 받아도 기본(ko)으로 동작
  }
  applyText();
  data = await invoke<MemoData>("memo_load");
  loaded = true;
  textEl.value = data.tabs[data.active];
  applyAlpha(data.alpha);
  renderTabs();
  textEl.focus();
})();
void listen<string>("language-changed", (event) => {
  setLang(event.payload);
  applyText();
});
