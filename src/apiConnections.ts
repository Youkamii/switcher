import { invoke } from "@tauri-apps/api/core";
import { currentLang } from "./i18n";

const messages = {
  ko: {
    title: "API 연결", add: "API 키 등록", name: "연결 이름 (영문·숫자·-·_)", key: "API 키", save: "키 저장", cancel: "취소",
    billing: "API 사용은 구독과 별도 과금됩니다. 실행 명령을 터미널에 붙여넣을 때 이 키를 사용합니다.",
    saved: "키 저장됨 · 연결 확인 전", check: "연결 확인", checked: "모델 목록 조회 성공 · 생성 권한·잔액은 별도 확인", checking: "확인 중…", copy: "실행 명령 복사", copied: "명령을 복사했습니다. 사용할 프로젝트의 터미널에 붙여넣으세요.",
    remove: "삭제", confirm: "삭제 확인", failed: "복사 실패 — 아래 명령을 직접 복사하세요", noQuota: "API · 별도 과금", replace: "같은 이름으로 저장하면 기존 키를 교체합니다.",
  },
  en: {
    title: "API connections", add: "Add API key", name: "Name (letters, numbers, - or _)", key: "API key", save: "Save key", cancel: "Cancel",
    billing: "API usage is billed separately from subscriptions. This key is used when you run the copied command in your terminal.",
    saved: "Key saved · not checked", check: "Check connection", checked: "Models listed · generation access and balance not checked", checking: "Checking…", copy: "Copy launch command", copied: "Command copied. Paste it into your project terminal.",
    remove: "Delete", confirm: "Confirm delete", failed: "Copy failed — copy the command below", noQuota: "API · separate billing", replace: "Saving the same name replaces its key.",
  },
};
export function apiText() { return currentLang() === "ko" ? messages.ko : messages.en; }
let apiPending = 0;
export function apiIsBusy(): boolean { return apiPending > 0 || document.querySelector(".api-form") !== null; }
type Connection = { provider: string; name: string };
type Hooks = { changed: () => Promise<void>; fit: () => void; toast: (message: string, error?: boolean) => void };

export async function apiConnections(provider: string, hooks: Hooks): Promise<HTMLElement> {
  const text = apiText();
  const panel = document.createElement("details");
  panel.className = "api-panel";
  const title = document.createElement("summary");
  title.textContent = text.title;
  panel.append(title);
  panel.addEventListener("toggle", hooks.fit);
  const connections = await invoke<Connection[]>("api_list", { provider });
  title.textContent += ` · ${connections.length}`;
  const note = document.createElement("p");
  note.className = "help";
  note.textContent = text.billing;
  panel.append(note);
  for (const connection of connections) {
    const card = document.createElement("div");
    card.className = "api-card";
    const head = document.createElement("div");
    head.className = "card-head";
    const name = document.createElement("strong");
    name.className = "card-name";
    name.textContent = connection.name;
    const badge = document.createElement("span");
    badge.className = "api-badge";
    badge.textContent = "API";
    head.append(name, badge);
    const status = document.createElement("p");
    status.className = "help";
    status.textContent = text.saved;
    status.setAttribute("role", "status");
    const actions = document.createElement("div");
    actions.className = "api-actions";
    const check = document.createElement("button");
    check.textContent = text.check;
    check.onclick = async () => {
      check.disabled = true;
      apiPending++;
      status.textContent = text.checking;
      try {
        await invoke("api_check", connection);
        status.textContent = text.checked;
      } catch (error) { status.textContent = String(error); }
      finally { check.disabled = false; apiPending--; hooks.fit(); }
    };
    const copy = document.createElement("button");
    copy.textContent = text.copy;
    copy.onclick = async () => {
      copy.disabled = true;
      apiPending++;
      try {
        const command = await invoke<string>("api_launch_command", connection);
        card.querySelector("textarea")?.remove();
        const fallback = document.createElement("textarea");
        fallback.readOnly = true;
        fallback.value = command;
        fallback.setAttribute("aria-label", text.copy);
        card.append(fallback);
        // Clipboard permission may remain pending in an inactive WKWebView. The
        // command is already usable; never hold the whole renderer on that promise.
        void navigator.clipboard.writeText(command).then(
          () => hooks.toast(text.copied),
          () => { if (fallback.isConnected) fallback.select(); hooks.toast(text.failed, true); },
        );
      } catch (error) { hooks.toast(String(error), true); }
      finally { copy.disabled = false; apiPending--; hooks.fit(); }
    };
    const remove = document.createElement("button");
    remove.textContent = text.remove;
    let armedAt = 0;
    const disarm = () => { armedAt = 0; remove.textContent = text.remove; remove.classList.remove("danger-armed"); };
    card.addEventListener("pointerleave", disarm);
    card.addEventListener("focusout", (event) => { if (!card.contains(event.relatedTarget as Node | null)) disarm(); });
    remove.onclick = async () => {
      if (!armedAt || performance.now() - armedAt > 3000) {
        armedAt = performance.now(); remove.textContent = text.confirm; remove.classList.add("danger-armed"); return;
      }
      remove.disabled = true;
      apiPending++;
      try { await invoke("api_delete", connection); card.remove(); title.textContent = `${text.title} · ${panel.querySelectorAll(".api-card").length}`; }
      catch (error) { hooks.toast(String(error), true); }
      finally { apiPending--; disarm(); remove.disabled = false; hooks.fit(); }
    };
    actions.append(copy, check, remove);
    card.append(head, status, actions);
    panel.append(card);
  }
  const add = document.createElement("button");
  add.className = "api-add";
  add.textContent = text.add;
  add.onclick = () => {
    if (panel.querySelector("form")) return;
    add.hidden = true;
    const form = document.createElement("form");
    form.className = "api-form";
    const nameLabel = document.createElement("label");
    nameLabel.textContent = text.name;
    const name = document.createElement("input");
    name.required = true; name.maxLength = 32; name.pattern = "[A-Za-z0-9_-]+"; name.autocomplete = "off";
    nameLabel.append(name);
    const keyLabel = document.createElement("label");
    keyLabel.textContent = text.key;
    const key = document.createElement("input");
    key.type = "password"; key.required = true; key.maxLength = 4096; key.autocomplete = "new-password"; key.spellcheck = false;
    keyLabel.append(key);
    const help = document.createElement("p"); help.className = "help"; help.textContent = text.replace;
    const save = document.createElement("button"); save.type = "submit"; save.className = "primary"; save.textContent = text.save;
    const cancel = document.createElement("button"); cancel.type = "button"; cancel.textContent = text.cancel;
    const close = () => { key.value = ""; form.remove(); add.hidden = false; hooks.fit(); };
    cancel.onclick = close;
    form.addEventListener("keydown", (event) => { if (event.key === "Escape" && !save.disabled) close(); });
    form.onsubmit = async (event) => {
      event.preventDefault();
      if (save.disabled) return;
      apiPending++;
      save.disabled = true; cancel.disabled = true; name.readOnly = true; key.readOnly = true;
      try {
        await invoke("api_save", { provider, name: name.value.trim(), key: key.value.trim() });
        close(); hooks.toast(text.saved);
      } catch (error) { hooks.toast(String(error), true); }
      finally { apiPending--; key.value = ""; save.disabled = false; cancel.disabled = false; name.readOnly = false; key.readOnly = false; }
      if (!form.isConnected) await hooks.changed();
    };
    form.append(nameLabel, keyLabel, help, save, cancel);
    panel.append(form); name.focus(); hooks.fit();
  };
  panel.append(add);
  return panel;
}
