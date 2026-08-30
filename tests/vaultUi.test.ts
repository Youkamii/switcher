import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  reconcileVaultProfileChoices,
  selectedVaultProfiles,
  vaultBoundaryPolicy,
  vaultInteractionLocked,
} from "../src/vaultSelection.ts";

const html = readFileSync(new URL("../vault.html", import.meta.url), "utf8");
const source = readFileSync(new URL("../src/vault.ts", import.meta.url), "utf8");
const selectionSource = readFileSync(
  new URL("../src/vaultSelection.ts", import.meta.url),
  "utf8",
);
const css = readFileSync(new URL("../src/vault.css", import.meta.url), "utf8");
const viteConfig = readFileSync(new URL("../vite.config.ts", import.meta.url), "utf8");
const packageJson = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url), "utf8"),
) as { dependencies?: Record<string, string> };

test("builds an alias-only export DTO and preserves each email-display choice", () => {
  const selections = selectedVaultProfiles([
    {
      profile: { provider: "claude", name: "work", active: true, revision: 1 },
      selected: true,
      hideEmail: true,
    },
    {
      profile: { provider: "codex", name: "personal", active: false, revision: 2 },
      selected: false,
      hideEmail: false,
    },
  ]);

  assert.deepEqual(selections, [
    { provider: "claude", name: "work", hideEmail: true },
  ]);
  assert.equal(JSON.stringify(selections).includes("email"), false);
  assert.deepEqual(selectedVaultProfiles([]), []);
});

test("keeps Vault choices across focus refreshes while replacing live profile state", () => {
  const previous = [
    {
      profile: { provider: "claude", name: "work", active: false, revision: 7 },
      selected: true,
      hideEmail: false,
    },
    {
      profile: { provider: "codex", name: "removed", active: true, revision: 3 },
      selected: true,
      hideEmail: true,
    },
  ] as const;
  const profiles = [
    { provider: "claude", name: "work", active: true, revision: 7 },
    { provider: "codex", name: "new", active: false, revision: 1 },
  ] as const;

  assert.deepEqual(reconcileVaultProfileChoices(profiles, previous), [
    {
      profile: profiles[0],
      selected: true,
      hideEmail: false,
    },
    {
      profile: profiles[1],
      selected: false,
      hideEmail: true,
    },
  ]);
  assert.deepEqual(previous, [
    {
      profile: { provider: "claude", name: "work", active: false, revision: 7 },
      selected: true,
      hideEmail: false,
    },
    {
      profile: { provider: "codex", name: "removed", active: true, revision: 3 },
      selected: true,
      hideEmail: true,
    },
  ]);
});

test("resets Vault choices when an alias is recreated for a different profile revision", () => {
  const previous = [
    {
      profile: { provider: "claude", name: "work", active: true, revision: 7 },
      selected: true,
      hideEmail: false,
    },
  ] as const;
  const profiles = [
    { provider: "claude", name: "work", active: false, revision: 8 },
  ] as const;

  assert.deepEqual(reconcileVaultProfileChoices(profiles, previous), [
    {
      profile: profiles[0],
      selected: false,
      hideEmail: true,
    },
  ]);
  assert.equal(previous[0].selected, true);
  assert.equal(previous[0].hideEmail, false);
});

test("blocks lifecycle boundaries while busy and keeps recovery available across forced hiding", () => {
  assert.equal(vaultInteractionLocked(false, false), false);
  assert.equal(vaultInteractionLocked(true, false), true);
  assert.equal(vaultInteractionLocked(false, true), true);
  assert.equal(vaultInteractionLocked(true, true), true);

  for (const boundary of ["tab", "close", "hidden"] as const) {
    assert.deepEqual(vaultBoundaryPolicy(true, boundary), {
      allowed: false,
      clearRecovery: false,
      clearImport: false,
    });
  }

  assert.deepEqual(vaultBoundaryPolicy(false, "hidden"), {
    allowed: true,
    clearRecovery: true,
    clearImport: true,
  });
  for (const boundary of ["tab", "close"] as const) {
    assert.deepEqual(vaultBoundaryPolicy(false, boundary), {
      allowed: true,
      clearRecovery: true,
      clearImport: true,
    });
  }

  assert.deepEqual(
    vaultBoundaryPolicy(vaultInteractionLocked(false, true), "close"),
    {
      allowed: false,
      clearRecovery: false,
      clearImport: false,
    },
  );
});

test("keeps recovery pending until the user copies or explicitly confirms storage", () => {
  const exportFlow = source.slice(
    source.indexOf("async function exportVault"),
    source.indexOf("async function copyRecoveryCode"),
  );
  const displayedAt = exportFlow.indexOf("showRecoveryResult(result.recovery_code)");
  assert.ok(displayedAt >= 0);
  assert.doesNotMatch(exportFlow, /acknowledgeStoredRecovery/);

  const restoreFlow = source.slice(
    source.indexOf("async function restorePendingRecovery"),
    source.indexOf("function refreshVisibleVault"),
  );
  assert.match(restoreFlow, /invoke<string \| null>\("vault_pending_recovery"\)/);
  assert.ok(restoreFlow.indexOf("showRecoveryResult(code)") >= 0);
  assert.doesNotMatch(restoreFlow, /acknowledgeStoredRecovery/);
  assert.match(restoreFlow, /catch \{\s*recoveryPendingAck = true;/);

  const copyFlow = source.slice(
    source.indexOf("async function copyRecoveryCode"),
    source.indexOf("async function confirmRecoveryStored"),
  );
  assert.match(copyFlow, /if \(copied && recoveryPendingAck\)/);
  assert.match(copyFlow, /await acknowledgeStoredRecovery\(code\)/);
  const confirmFlow = source.slice(
    source.indexOf("async function confirmRecoveryStored"),
    source.indexOf("async function chooseVaultFile"),
  );
  assert.match(confirmFlow, /await acknowledgeStoredRecovery\(code\)/);
  assert.match(confirmFlow, /recoveryPendingAck = !acknowledged/);
  assert.match(html, /id="confirm-recovery"/);
  assert.match(
    source,
    /copyRecovery\.disabled = next \|\| !recoveryCode\.textContent;\s*confirmRecovery\.disabled = next \|\| !recoveryCode\.textContent \|\| !recoveryPendingAck;/,
  );
  assert.match(source, /visibilityState === "hidden"[\s\S]*?refreshVisibleVault\(\)/);
  assert.match(source, /addEventListener\("focus"[\s\S]*?refreshVisibleVault\(\)/);
  assert.match(source, /applyText\(\);\s*await refreshVisibleVault\(\);/);
});

test("uses a dedicated themed two-tab window with accessible status regions", () => {
  assert.match(viteConfig, /vault:\s*"vault\.html"/);
  assert.match(html, /src\/theme\.css/);
  assert.match(html, /src\/vault\.css/);
  assert.match(html, /src\/vault\.ts/);
  assert.match(html, /id="export-tab"[\s\S]*role="tab"/);
  assert.match(html, /id="import-tab"[\s\S]*role="tab"/);
  assert.match(html, /id="export-status"[^>]*aria-live="polite"/);
  assert.match(html, /id="import-status"[^>]*aria-live="polite"/);
  assert.match(html, /id="recovery-input"[\s\S]*type="password"/);
  assert.match(css, /\.vault-panel\[hidden\]/);
});

test("loads only the alias profile DTO and reconciles refreshed choices", () => {
  assert.match(
    selectionSource,
    /interface VaultProfile \{\s*provider: string;\s*name: string;\s*active: boolean;\s*revision: number;/,
  );
  assert.match(source, /invoke<VaultProfile\[]>\("vault_list_profiles"\)/);
  assert.match(
    source,
    /choices = reconcileVaultProfileChoices\(profiles, choices\)/,
  );
  assert.doesNotMatch(source, /profile\.(?:email|id|accountUuid|organizationUuid)/);
  assert.doesNotMatch(selectionSource, /\b(?:email|id|accountUuid|organizationUuid)\s*:/);
  assert.match(source, /if \(selections\.length === 0\)[\s\S]*?noSelection/);
});

test("uses native filtered dialogs and treats cancellation as a quiet return", () => {
  assert.equal(packageJson.dependencies?.["@tauri-apps/plugin-dialog"]?.startsWith("^2"), true);
  assert.match(source, /import \{ open, save \} from "@tauri-apps\/plugin-dialog"/);
  assert.match(
    source,
    /save\(\{[\s\S]*?filters: \[\{ name: "Switcher Vault", extensions: \["switcher-vault"\] \}\][\s\S]*?\}\);/,
  );
  assert.match(
    source,
    /open\(\{[\s\S]*?multiple: false,[\s\S]*?directory: false,[\s\S]*?extensions: \["switcher-vault"\]/,
  );
  assert.match(source, /if \(!path\) \{\s*setBusy\(false\);\s*return;/);
  assert.match(
    source,
    /if \(typeof selected !== "string"\) \{\s*setBusy\(false\);\s*return;\s*\}/,
  );
});

test("clears recovery secrets at explicit boundaries without discarding completed exports", () => {
  assert.match(
    source,
    /const pending = invoke<VaultImportResult>\("vault_import", \{[\s\S]*?recoveryCode: recoveryCodeValue,[\s\S]*?\}\);\s*recoveryInput\.value = "";\s*\n\s*try \{/,
    "the password field must clear immediately after starting the native import",
  );
  assert.match(source, /finally \{\s*recoveryInput\.value = "";/);
  assert.match(source, /function switchTab[\s\S]*?applyBoundary\("tab"\)/);
  assert.match(source, /async function exportVault[\s\S]*?clearRecoveryResult\(\);/);
  const hideFlow = source.slice(
    source.indexOf("async function hideSelf"),
    source.indexOf("function applyText"),
  );
  assert.match(hideFlow, /await invoke<void>\("vault_hide"\)/);
  assert.ok(hideFlow.indexOf('applyBoundary("close")') > hideFlow.indexOf('invoke<void>("vault_hide")'));
  assert.match(hideFlow, /catch \{\s*recoveryPendingAck = true;[\s\S]*?restorePendingRecovery\(\)/);
  assert.doesNotMatch(source, /\.hide\(\)/);
  assert.match(source, /visibilityState === "hidden"[\s\S]*?applyBoundary\("hidden"\)/);
  assert.doesNotMatch(source, /viewGeneration|generation !==/);
});

test("does not persist or echo a recovery code or native file path", () => {
  assert.doesNotMatch(source, /localStorage|sessionStorage|console\./);
  assert.doesNotMatch(
    source,
    /importFileState\.textContent\s*=\s*(?:selected|path|importPath)\s*;/,
  );
  assert.doesNotMatch(source, /setStatus\([^\n]*?(?:path|recoveryCodeValue)/);
  assert.match(source, /catch \{\s*setStatus\(importStatus, vt\("importFailed"\), "error"\);/);
  assert.match(source, /catch \{[\s\S]*?setStatus\(exportStatus, vt\("exportFailed"\), "error"\);/);
});

test("shows a restart warning when import safety-marker cleanup remains", () => {
  assert.match(source, /interface VaultImportResult \{[\s\S]*?cleanup_pending: boolean;/);
  assert.match(
    source,
    /if \(result\.cleanup_pending\) \{\s*setStatus\(importStatus, vt\("importCleanupPending"\), "warning"\);/,
  );
  assert.match(
    source,
    /가져오기 완료, 안전 표식 정리를 위해 앱을 다시 시작하세요\./,
  );
  assert.match(css, /\.status\.warning/);
});

test("prevents duplicate actions and follows live language and accent settings", () => {
  assert.match(
    source,
    /function setBusy[\s\S]*?exportTab\.disabled[\s\S]*?importTab\.disabled[\s\S]*?closeButton\.disabled[\s\S]*?exportButton\.disabled[\s\S]*?importButton\.disabled/,
  );
  assert.match(source, /async function exportVault\(\) \{\s*if \(interactionsLocked\(\)\) return;/);
  assert.match(source, /async function importVault\(\) \{\s*if \(interactionsLocked\(\)\) return;/);
  assert.match(source, /get_language/);
  assert.match(source, /language-changed/);
  assert.match(source, /get_accent_theme/);
  assert.match(source, /accent-theme-changed/);
  assert.match(source, /applyAccentTheme/);
  assert.match(source, /event\.key === "Escape"[\s\S]*?hideSelf/);
});

test("explains the exact limit of email hiding", () => {
  assert.match(
    source,
    /이메일 숨김은 Switcher의 메타정보와 표시에서만 적용됩니다\. 토큰 자체의 식별정보까지 받는 사람에게 숨길 수는 없습니다\./,
  );
  assert.match(
    source,
    /Email hiding applies only to Switcher metadata and display\.[\s\S]*?identifiers contained in the token itself/,
  );
});
