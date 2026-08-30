import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { currentLang, setLang } from "./i18n";
import { applyAccentTheme } from "./theme";
import {
  reconcileVaultProfileChoices,
  selectedVaultProfiles,
  vaultBoundaryPolicy,
  vaultInteractionLocked,
  type VaultBoundary,
  type VaultProfile,
  type VaultProfileChoice,
} from "./vaultSelection";

interface VaultExportResult {
  recovery_code: string;
  exported: number;
}

interface VaultImportResult {
  imported: number;
  skipped: number;
  cleanup_pending: boolean;
}

type VaultTextKey = keyof typeof VAULT_TEXT.ko;

const VAULT_TEXT = {
  ko: {
    title: "인증정보 이동",
    close: "닫기",
    tabLabel: "인증정보 이동 방식",
    exportTab: "내보내기",
    importTab: "가져오기",
    exportIntro:
      "이 PC에 저장된 프로필 중 옮길 항목을 고르세요. 내보내는 동안 새 Claude 로그인을 시작하지 마세요.",
    loadingProfiles: "저장된 프로필을 불러오는 중…",
    noProfiles: "내보낼 Claude 또는 Codex 프로필이 없습니다.",
    loadProfilesFailed: "저장된 프로필을 불러오지 못했습니다.",
    active: "현재 사용 중",
    hideEmail: "받는 앱에서 이메일 숨김",
    exportButton: "암호화 파일 내보내기",
    noSelection: "내보낼 프로필을 하나 이상 선택하세요.",
    exportDialogTitle: "암호화 인증정보 파일 저장",
    exportWorking: "선택한 프로필을 암호화하는 중…",
    exportDone: "프로필 {n}개를 내보냈습니다. 복구 코드는 지금 따로 보관하세요.",
    exportFailed: "암호화 파일을 만들지 못했습니다.",
    recoveryTitle: "복구 코드",
    recoveryOnce: "파일과 따로 보관한 뒤 복사 또는 보관 완료를 눌러 확인하세요.",
    copy: "복사",
    confirmStored: "보관 완료",
    copied: "복구 코드를 복사했습니다.",
    recoveryStored: "복구 코드 보관을 확인했습니다.",
    copyFailed: "복사하지 못했습니다. 표시된 코드를 직접 복사하세요.",
    recoveryRestored: "보류 중이던 복구 코드를 다시 표시했습니다. 지금 별도로 보관하세요.",
    recoveryAckFailed:
      "복구 코드 보관 확인을 처리하지 못했습니다. 코드를 복사하거나 따로 적어 둔 뒤 다시 시도하세요.",
    recoveryCheckFailed: "보류 중인 복구 코드가 있는지 확인하지 못했습니다.",
    importIntro: "암호화 파일과 별도로 받은 복구 코드가 모두 필요합니다.",
    chooseFile: ".switcher-vault 파일 선택",
    importDialogTitle: "암호화 인증정보 파일 선택",
    noFileSelected: "선택된 파일 없음",
    fileSelected: "암호화 파일이 선택되었습니다.",
    recoveryInput: "복구 코드",
    recoveryPlaceholder: "별도로 받은 복구 코드 입력",
    emailNotice:
      "이메일 숨김은 Switcher의 메타정보와 표시에서만 적용됩니다. 토큰 자체의 식별정보까지 받는 사람에게 숨길 수는 없습니다.",
    importButton: "새 프로필로 가져오기",
    chooseFileFirst: "가져올 암호화 파일을 먼저 선택하세요.",
    enterRecoveryCode: "복구 코드를 입력하세요.",
    importWorking: "암호화 파일을 확인하고 새 프로필로 넣는 중…",
    importDone: "프로필 {imported}개를 가져왔고 {skipped}개는 건너뛰었습니다.",
    importCleanupPending: "가져오기 완료, 안전 표식 정리를 위해 앱을 다시 시작하세요.",
    importFailed: "가져오지 못했습니다. 파일과 복구 코드를 확인하세요.",
    closeBlocked: "작업 또는 복구 코드 확인이 끝나지 않아 닫을 수 없습니다.",
  },
  en: {
    title: "Transfer credentials",
    close: "Close",
    tabLabel: "Credential transfer mode",
    exportTab: "Export",
    importTab: "Import",
    exportIntro:
      "Choose the saved profiles to transfer from this PC. Do not start a new Claude login during export.",
    loadingProfiles: "Loading saved profiles…",
    noProfiles: "There are no saved Claude or Codex profiles to export.",
    loadProfilesFailed: "Could not load the saved profiles.",
    active: "Active",
    hideEmail: "Hide email in the receiving app",
    exportButton: "Export encrypted file",
    noSelection: "Select at least one profile to export.",
    exportDialogTitle: "Save encrypted credentials",
    exportWorking: "Encrypting the selected profiles…",
    exportDone: "Exported {n} profiles. Store the recovery code separately now.",
    exportFailed: "Could not create the encrypted file.",
    recoveryTitle: "Recovery code",
    recoveryOnce: "Store this separately from the file, then copy or confirm that it is saved.",
    copy: "Copy",
    confirmStored: "I've stored it",
    copied: "Recovery code copied.",
    recoveryStored: "Recovery code storage confirmed.",
    copyFailed: "Could not copy. Copy the displayed code manually.",
    recoveryRestored: "Restored the pending recovery code. Store it separately now.",
    recoveryAckFailed:
      "Could not confirm recovery-code storage. Copy it or write it down, then try again.",
    recoveryCheckFailed: "Could not check for a pending recovery code.",
    importIntro: "You need both the encrypted file and its separately shared recovery code.",
    chooseFile: "Choose .switcher-vault file",
    importDialogTitle: "Choose encrypted credentials",
    noFileSelected: "No file selected",
    fileSelected: "Encrypted file selected.",
    recoveryInput: "Recovery code",
    recoveryPlaceholder: "Enter the separately shared recovery code",
    emailNotice:
      "Email hiding applies only to Switcher metadata and display. It cannot hide identifiers contained in the token itself from the recipient.",
    importButton: "Import as new profiles",
    chooseFileFirst: "Choose the encrypted file to import first.",
    enterRecoveryCode: "Enter the recovery code.",
    importWorking: "Checking the encrypted file and adding new profiles…",
    importDone: "Imported {imported} profiles and skipped {skipped}.",
    importCleanupPending: "Import complete. Restart the app to finish cleaning up safety markers.",
    importFailed: "Import failed. Check the file and recovery code.",
    closeBlocked: "Finish the transfer or confirm recovery-code storage before closing.",
  },
} as const;

const exportTab = document.getElementById("export-tab") as HTMLButtonElement;
const importTab = document.getElementById("import-tab") as HTMLButtonElement;
const closeButton = document.getElementById("vault-close") as HTMLButtonElement;
const exportPanel = document.getElementById("export-panel") as HTMLElement;
const importPanel = document.getElementById("import-panel") as HTMLElement;
const profileList = document.getElementById("profile-list") as HTMLElement;
const exportButton = document.getElementById("export-button") as HTMLButtonElement;
const exportStatus = document.getElementById("export-status") as HTMLElement;
const recoveryResult = document.getElementById("recovery-result") as HTMLElement;
const recoveryCode = document.getElementById("recovery-code") as HTMLElement;
const copyRecovery = document.getElementById("copy-recovery") as HTMLButtonElement;
const confirmRecovery = document.getElementById("confirm-recovery") as HTMLButtonElement;
const chooseImportFile = document.getElementById("choose-import-file") as HTMLButtonElement;
const importFileState = document.getElementById("import-file-state") as HTMLElement;
const recoveryInput = document.getElementById("recovery-input") as HTMLInputElement;
const importButton = document.getElementById("import-button") as HTMLButtonElement;
const importStatus = document.getElementById("import-status") as HTMLElement;

let choices: VaultProfileChoice[] = [];
let importPath: string | null = null;
let busy = false;
let recoveryPendingAck = false;
let visibleRefresh: Promise<void> | null = null;
let accentThemeFromEvent = false;

function vt(key: VaultTextKey, params?: Record<string, string | number>): string {
  const language = currentLang() === "ko" ? "ko" : "en";
  let text: string = VAULT_TEXT[language][key];
  if (params) {
    for (const [name, value] of Object.entries(params)) {
      text = text.split(`{${name}}`).join(String(value));
    }
  }
  return text;
}

function setStatus(
  element: HTMLElement,
  message = "",
  kind: "idle" | "success" | "warning" | "error" = "idle",
) {
  element.textContent = message;
  element.classList.toggle("success", kind === "success");
  element.classList.toggle("warning", kind === "warning");
  element.classList.toggle("error", kind === "error");
}

function clearRecoveryResult() {
  recoveryCode.textContent = "";
  recoveryResult.hidden = true;
  copyRecovery.disabled = true;
  confirmRecovery.disabled = true;
}

function clearImportState() {
  recoveryInput.value = "";
  importPath = null;
  importFileState.textContent = vt("noFileSelected");
  syncImportButton();
}

function applyBoundary(boundary: VaultBoundary): boolean {
  const policy = vaultBoundaryPolicy(
    vaultInteractionLocked(busy, recoveryPendingAck),
    boundary,
  );
  if (!policy.allowed) return false;
  if (policy.clearRecovery) clearRecoveryResult();
  if (policy.clearImport) clearImportState();
  return true;
}

function setBusy(next: boolean) {
  busy = next;
  const locked = vaultInteractionLocked(busy, recoveryPendingAck);
  exportTab.disabled = locked;
  importTab.disabled = locked;
  closeButton.disabled = locked;
  exportTab.setAttribute("aria-disabled", String(locked));
  importTab.setAttribute("aria-disabled", String(locked));
  closeButton.setAttribute("aria-disabled", String(locked));
  exportButton.disabled = locked || selectedVaultProfiles(choices).length === 0;
  chooseImportFile.disabled = locked;
  importButton.disabled = locked || !importPath || recoveryInput.value.trim().length === 0;
  recoveryInput.disabled = locked;
  copyRecovery.disabled = next || !recoveryCode.textContent;
  confirmRecovery.disabled = next || !recoveryCode.textContent || !recoveryPendingAck;
  profileList.querySelectorAll<HTMLInputElement>("input").forEach((input) => {
    input.disabled = locked;
  });
}

function interactionsLocked(): boolean {
  return vaultInteractionLocked(busy, recoveryPendingAck);
}

function syncImportButton() {
  importButton.disabled =
    interactionsLocked() || !importPath || recoveryInput.value.trim().length === 0;
}

function showRecoveryResult(code: string) {
  recoveryCode.textContent = code;
  recoveryResult.hidden = false;
  recoveryPendingAck = true;
}

async function acknowledgeStoredRecovery(code: string): Promise<boolean> {
  try {
    return await invoke<boolean>("vault_ack_recovery_stored", {
      recoveryCode: code,
    });
  } catch {
    return false;
  }
}

async function restorePendingRecovery() {
  if (busy) return;
  const wasPendingAck = recoveryPendingAck;
  setBusy(true);
  try {
    const code = await invoke<string | null>("vault_pending_recovery");
    if (!code) {
      recoveryPendingAck = false;
      if (wasPendingAck && recoveryCode.textContent) {
        setStatus(exportStatus, vt("recoveryRestored"), "success");
      }
      return;
    }

    showRecoveryResult(code);
    setStatus(exportStatus, vt("recoveryRestored"), "success");
  } catch {
    recoveryPendingAck = true;
    setStatus(exportStatus, vt("recoveryCheckFailed"), "error");
  } finally {
    setBusy(false);
  }
}

function refreshVisibleVault(): Promise<void> {
  if (visibleRefresh) return visibleRefresh;
  if (busy) return Promise.resolve();

  const refresh = (async () => {
    await restorePendingRecovery();
    if (!busy && !recoveryPendingAck) await loadProfiles();
  })();
  visibleRefresh = refresh;
  void refresh.finally(() => {
    if (visibleRefresh === refresh) visibleRefresh = null;
  });
  return refresh;
}

function profileProviderLabel(provider: string): string {
  if (provider.toLowerCase() === "claude") return "Claude";
  if (provider.toLowerCase() === "codex") return "Codex";
  return provider;
}

function renderProfiles() {
  const locked = interactionsLocked();
  profileList.replaceChildren();
  profileList.setAttribute("aria-busy", "false");

  if (choices.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.textContent = vt("noProfiles");
    profileList.appendChild(empty);
    exportButton.disabled = true;
    return;
  }

  choices.forEach((choice, index) => {
    const row = document.createElement("div");
    row.className = "profile-row";

    const profileLabel = document.createElement("label");
    profileLabel.className = "profile-choice";

    const select = document.createElement("input");
    select.type = "checkbox";
    select.checked = choice.selected;
    select.disabled = locked;
    select.addEventListener("change", () => {
      choices[index].selected = select.checked;
      exportButton.disabled =
        interactionsLocked() || selectedVaultProfiles(choices).length === 0;
      setStatus(exportStatus);
    });

    const provider = document.createElement("span");
    provider.className = "provider-badge";
    provider.textContent = profileProviderLabel(choice.profile.provider);

    const name = document.createElement("span");
    name.className = "profile-name";
    name.textContent = choice.profile.name;

    profileLabel.append(select, provider, name);
    row.appendChild(profileLabel);

    if (choice.profile.active) {
      const active = document.createElement("span");
      active.className = "active-badge";
      active.textContent = vt("active");
      row.appendChild(active);
    }

    const hideLabel = document.createElement("label");
    hideLabel.className = "hide-choice";
    const hide = document.createElement("input");
    hide.type = "checkbox";
    hide.checked = choice.hideEmail;
    hide.disabled = locked;
    hide.addEventListener("change", () => {
      choices[index].hideEmail = hide.checked;
    });
    const hideText = document.createElement("span");
    hideText.textContent = vt("hideEmail");
    hideLabel.append(hide, hideText);
    row.appendChild(hideLabel);

    profileList.appendChild(row);
  });

  exportButton.disabled = locked || selectedVaultProfiles(choices).length === 0;
}

async function loadProfiles() {
  profileList.setAttribute("aria-busy", "true");
  profileList.textContent = vt("loadingProfiles");
  exportButton.disabled = true;
  try {
    const profiles = await invoke<VaultProfile[]>("vault_list_profiles");
    choices = reconcileVaultProfileChoices(profiles, choices);
    renderProfiles();
  } catch {
    profileList.setAttribute("aria-busy", "false");
    profileList.textContent = vt("loadProfilesFailed");
    exportButton.disabled = true;
  }
}

function switchTab(tab: "export" | "import") {
  const showExport = tab === "export";
  const alreadySelected = exportPanel.hidden === !showExport;
  if (alreadySelected) return;
  if (!applyBoundary("tab")) return;
  exportTab.classList.toggle("active", showExport);
  exportTab.setAttribute("aria-selected", String(showExport));
  importTab.classList.toggle("active", !showExport);
  importTab.setAttribute("aria-selected", String(!showExport));
  exportPanel.hidden = !showExport;
  importPanel.hidden = showExport;
  setStatus(exportStatus);
  setStatus(importStatus);
}

async function exportVault() {
  if (interactionsLocked()) return;
  const selections = selectedVaultProfiles(choices);
  if (selections.length === 0) {
    setStatus(exportStatus, vt("noSelection"), "error");
    return;
  }

  clearRecoveryResult();
  setStatus(exportStatus);
  setBusy(true);
  let path: string | null;
  try {
    path = await save({
      title: vt("exportDialogTitle"),
      defaultPath: "accounts.switcher-vault",
      filters: [{ name: "Switcher Vault", extensions: ["switcher-vault"] }],
    });
  } catch {
    setStatus(exportStatus, vt("exportFailed"), "error");
    setBusy(false);
    return;
  }
  if (!path) {
    setBusy(false);
    return;
  }
  clearRecoveryResult();
  setStatus(exportStatus, vt("exportWorking"));
  try {
    const result = await invoke<VaultExportResult>("vault_export", { path, selections });
    showRecoveryResult(result.recovery_code);
    setStatus(exportStatus, vt("exportDone", { n: result.exported }), "success");
  } catch {
    setStatus(exportStatus, vt("exportFailed"), "error");
  } finally {
    setBusy(false);
  }
}

async function copyRecoveryCode() {
  const code = recoveryCode.textContent;
  if (!code || busy) return;
  setBusy(true);
  let copied = false;
  try {
    await navigator.clipboard.writeText(code);
    copied = true;
  } catch {
    // The displayed code can still be copied manually.
  }

  if (copied && recoveryPendingAck) {
    const acknowledged = await acknowledgeStoredRecovery(code);
    recoveryPendingAck = !acknowledged;
    if (!acknowledged) {
      setStatus(exportStatus, vt("recoveryAckFailed"), "error");
      setBusy(false);
      return;
    }
  }

  setStatus(exportStatus, vt(copied ? "copied" : "copyFailed"), copied ? "success" : "error");
  setBusy(false);
}

async function confirmRecoveryStored() {
  const code = recoveryCode.textContent;
  if (!code || busy || !recoveryPendingAck) return;
  setBusy(true);
  const acknowledged = await acknowledgeStoredRecovery(code);
  recoveryPendingAck = !acknowledged;
  setStatus(
    exportStatus,
    vt(acknowledged ? "recoveryStored" : "recoveryAckFailed"),
    acknowledged ? "success" : "error",
  );
  setBusy(false);
}

async function chooseVaultFile() {
  if (interactionsLocked()) return;
  setStatus(importStatus);
  setBusy(true);
  let selected: string | null;
  try {
    selected = await open({
      title: vt("importDialogTitle"),
      multiple: false,
      directory: false,
      filters: [{ name: "Switcher Vault", extensions: ["switcher-vault"] }],
    });
  } catch {
    setStatus(importStatus, vt("importFailed"), "error");
    setBusy(false);
    return;
  }
  if (typeof selected !== "string") {
    setBusy(false);
    return;
  }
  importPath = selected;
  importFileState.textContent = vt("fileSelected");
  setStatus(importStatus);
  setBusy(false);
}

async function importVault() {
  if (interactionsLocked()) return;
  if (!importPath) {
    setStatus(importStatus, vt("chooseFileFirst"), "error");
    return;
  }
  const recoveryCodeValue = recoveryInput.value.trim();
  if (!recoveryCodeValue) {
    setStatus(importStatus, vt("enterRecoveryCode"), "error");
    return;
  }

  const path = importPath;
  setBusy(true);
  setStatus(importStatus, vt("importWorking"));
  const pending = invoke<VaultImportResult>("vault_import", {
    path,
    recoveryCode: recoveryCodeValue,
  });
  recoveryInput.value = "";

  try {
    const result = await pending;
    importPath = null;
    importFileState.textContent = vt("noFileSelected");
    if (result.cleanup_pending) {
      setStatus(importStatus, vt("importCleanupPending"), "warning");
    } else {
      setStatus(
        importStatus,
        vt("importDone", { imported: result.imported, skipped: result.skipped }),
        "success",
      );
    }
    await loadProfiles();
  } catch {
    setStatus(importStatus, vt("importFailed"), "error");
  } finally {
    recoveryInput.value = "";
    setBusy(false);
  }
}

async function hideSelf() {
  if (busy) return;
  setBusy(true);
  try {
    await invoke<void>("vault_hide");
    recoveryPendingAck = false;
    setBusy(false);
    if (applyBoundary("close")) {
      setStatus(exportStatus);
      setStatus(importStatus);
    }
  } catch {
    recoveryPendingAck = true;
    setBusy(false);
    setStatus(exportStatus, vt("closeBlocked"), "error");
    await restorePendingRecovery();
  }
}

function applyText() {
  document.documentElement.lang = currentLang() === "ko" ? "ko" : "en";
  document.title = vt("title");
  document.getElementById("vault-title")!.textContent = vt("title");
  document.getElementById("vault-close")!.setAttribute("aria-label", vt("close"));
  document.querySelector(".vault-tabs")!.setAttribute("aria-label", vt("tabLabel"));
  exportTab.textContent = vt("exportTab");
  importTab.textContent = vt("importTab");
  document.getElementById("export-intro")!.textContent = vt("exportIntro");
  document.getElementById("export-email-notice")!.textContent = vt("emailNotice");
  exportButton.textContent = vt("exportButton");
  document.getElementById("recovery-title")!.textContent = vt("recoveryTitle");
  document.getElementById("recovery-once")!.textContent = vt("recoveryOnce");
  copyRecovery.textContent = vt("copy");
  confirmRecovery.textContent = vt("confirmStored");
  document.getElementById("import-intro")!.textContent = vt("importIntro");
  chooseImportFile.textContent = vt("chooseFile");
  importFileState.textContent = importPath ? vt("fileSelected") : vt("noFileSelected");
  document.getElementById("recovery-input-label")!.textContent = vt("recoveryInput");
  recoveryInput.placeholder = vt("recoveryPlaceholder");
  document.getElementById("email-notice")!.textContent = vt("emailNotice");
  importButton.textContent = vt("importButton");
  renderProfiles();
}

exportTab.addEventListener("click", () => switchTab("export"));
importTab.addEventListener("click", () => switchTab("import"));
document.getElementById("vault-close")!.addEventListener("click", () => void hideSelf());
exportButton.addEventListener("click", () => void exportVault());
copyRecovery.addEventListener("click", () => void copyRecoveryCode());
confirmRecovery.addEventListener("click", () => void confirmRecoveryStored());
chooseImportFile.addEventListener("click", () => void chooseVaultFile());
recoveryInput.addEventListener("input", syncImportButton);
importButton.addEventListener("click", () => void importVault());
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") void hideSelf();
});
window.addEventListener("pagehide", () => {
  applyBoundary("hidden");
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") {
    applyBoundary("hidden");
  } else if (!busy) {
    void refreshVisibleVault();
  }
});
window.addEventListener("focus", () => {
  if (!busy) void refreshVisibleVault();
});

void (async () => {
  try {
    setLang(await invoke<string>("get_language"));
  } catch {
    // 설정을 못 읽으면 i18n 기본값(한국어)을 사용한다.
  }
  try {
    const saved = await invoke<string>("get_accent_theme");
    if (!accentThemeFromEvent) applyAccentTheme(saved);
  } catch {
    // 설정을 못 읽으면 theme.css의 기본 보라색을 사용한다.
  }
  applyText();
  await refreshVisibleVault();
})();

void listen<string>("language-changed", (event) => {
  setLang(event.payload);
  applyText();
});
void listen<string>("accent-theme-changed", (event) => {
  applyAccentTheme(event.payload);
  accentThemeFromEvent = true;
});
