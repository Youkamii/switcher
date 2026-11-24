import { getCurrentWindow } from "@tauri-apps/api/window";
import { currentLang } from "./i18n";
import { iconButton } from "./uiIcons";

// 창 안에 뜨는 대화상자 (다른 PC로 옮기기가 쓴다). 클래스 이름 `api-dialog`는 처음 이 틀을 쓰던
// API 키 등록 창에서 온 것이다 — 스타일과 화면 검사가 그 이름을 본다.
export type Hooks = { changed: () => Promise<void>; fit: () => void; toast: (message: string, error?: boolean) => void };

/** 대화상자가 열려 있다 — 그동안 로그인 시작·보기 전환을 막는다. */
export function dialogIsOpen(): boolean { return document.querySelector(".api-dialog") !== null; }

export function dialog(hooks: Hooks, title: string, canClose = () => true) {
  document.querySelector<HTMLDialogElement>(".api-dialog")?.dispatchEvent(new Event("api-dismiss"));
  const previousFocus = document.activeElement;
  const host = document.createElement("dialog"); host.className = "api-dialog";
  const head = document.createElement("div"); head.className = "api-dialog-head";
  head.title = currentLang() === "ko" ? "잡고 창 이동" : "Drag to move window";
  const heading = document.createElement("h2"); heading.id = "api-dialog-title"; heading.textContent = title;
  host.setAttribute("aria-labelledby", heading.id);
  const close = iconButton("close", (currentLang() === "ko" ? "취소" : "Cancel"), "api-close"); close.onclick = () => dismiss();
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
