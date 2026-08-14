import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const loginScrollRegion =
  /const interactionPanelOpen = loginOpen \|\| starPromptOpen;[\s\S]*?if \(interactionPanelOpen\) \{\s*const rect = visibleHitRect\(\s*app\.getBoundingClientRect\(\),\s*window\.innerWidth,\s*window\.innerHeight,?\s*\);\s*if \(rect\) \{\s*regions\.push\(\{ rect, action: null \}\);\s*hitElements\.push\(app\);/;

function loadVisibleHitRect() {
  const source = mainSource.match(/function visibleHitRect\([\s\S]*?\n\}/)?.[0];
  assert.ok(source, "visible hit-region geometry helper must exist");
  const javascript = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return Function(`${javascript}\nreturn visibleHitRect;`)() as (
    rect: { left: number; top: number; right: number; bottom: number },
    viewportWidth: number,
    viewportHeight: number,
  ) => [number, number, number, number] | null;
}

test("resizes the window after account login panels are mounted", () => {
  assert.match(
    mainSource,
    /function mountLoginPanel\([\s\S]*?loginHost\.replaceChildren\(panel\);[\s\S]*?fitHeight\(\);/,
    "every account login panel must request a height fit after mounting",
  );
});

test("keeps the active login panel when the account view type changes", () => {
  assert.doesNotMatch(
    mainSource,
    /async function render[\s\S]*?if \(loginOpen\)[\s\S]*?cancel_login[\s\S]*?github_login_cancel[\s\S]*?const mode = viewMode/,
    "rendering a new view must not cancel an active login",
  );
  assert.match(
    mainSource,
    /if \(loginOpen\) buffer\.appendChild\(loginHost\);\s*app\.replaceChildren\(buffer\);/,
    "the same login host node must survive app replacement",
  );
  assert.match(
    mainSource,
    /const width = loginOpen\s*\? 360\s*: starPromptOpen\s*\? 240\s*:/,
    "the login panel must retain the usable full width in Type 2 and Type 3",
  );
});

test("keeps cancel visible and clickable in locked account views", () => {
  assert.match(
    mainSource,
    /function beginLogin\([\s\S]*?waiting\.textContent = t\("gettingLoginUrl"\);[\s\S]*?cancelBtn\.textContent = t\("cancel"\);[\s\S]*?mountLoginPanel\(panel, attempt\);/,
    "cancel must be available while the login prompt is still being fetched",
  );
  assert.match(stylesSource, /#app\.locked #login-host \.login-panel[\s\S]*?display: block;/);
  assert.match(stylesSource, /#app\.locked #login-host button\.link[\s\S]*?display: inline-block;/);
  assert.match(
    mainSource,
    loginScrollRegion,
    "the native click-through map must include the visible scroll port during login",
  );
});

test("clips the login scroll hit-region to the visible WebView", () => {
  const visibleHitRect = loadVisibleHitRect();

  assert.deepEqual(
    visibleHitRect({ left: -12, top: 42, right: 372, bottom: 620 }, 360, 540),
    [0, 42, 360, 498],
  );
  assert.deepEqual(
    visibleHitRect({ left: 0, top: 30, right: 360, bottom: 540 }, 360, 540),
    [0, 30, 360, 510],
    "the titlebar above main must remain outside the scroll hit-region",
  );
  assert.equal(
    visibleHitRect({ left: 20, top: 550, right: 340, bottom: 620 }, 360, 540),
    null,
  );
});

test("keeps the full scroll viewport interactive while a transient panel is open", () => {
  assert.match(
    mainSource,
    /const interactionPanelOpen = loginOpen \|\| starPromptOpen;[\s\S]*?if \(locked\) \{[\s\S]*?if \(interactionPanelOpen\) \{\s*const rect = visibleHitRect\(\s*app\.getBoundingClientRect\(\)/,
    "the full main viewport must become a native UI region during login or the first-run prompt",
  );
  assert.doesNotMatch(
    mainSource,
    /querySelectorAll<HTMLElement>\([^)]*#app[^)]*\)/,
    "the regular selector list must not make the whole app interactive after login closes",
  );
  assert.match(stylesSource, /main \{[\s\S]*?min-height: 0;[\s\S]*?overflow-y: auto;/);
});

test("guards login completion and keeps rejected cancellation retryable", () => {
  assert.match(
    mainSource,
    /function finishLogin\(attempt: number\)[\s\S]*?if \(!isCurrentLogin\(attempt\)\) return;/,
    "an older async login must not close the current panel",
  );
  assert.match(
    mainSource,
    /async function cancelActiveLogin[\s\S]*?loginCancelingAttempt = attempt;[\s\S]*?try \{[\s\S]*?cancelExactSession[\s\S]*?catch \(error\) \{[\s\S]*?loginCancelingAttempt = null;[\s\S]*?button\.disabled = false;/,
    "an exact cancellation failure must leave the current panel retryable",
  );
});

test("routes GitHub wait and post-prompt cancel through the exact backend session", () => {
  assert.match(
    mainSource,
    /type GithubLoginPrompt = \{[\s\S]*?session_id: string;[\s\S]*?\};/,
    "the GitHub prompt must expose its backend session generation",
  );
  assert.match(
    mainSource,
    /watchGithubLogin\(prompt\.session_id, requestId, attempt\)/,
    "the GitHub waiter must identify the session it belongs to",
  );
  assert.match(
    mainSource,
    /invoke<boolean>\("github_login_cancel", \{\s*sessionId:/,
    "a GitHub cancel after the prompt must target that exact session",
  );
  assert.match(
    mainSource,
    /activeGithubRequestId = provider === "github" \? crypto\.randomUUID\(\) : null;/,
    "each GitHub login attempt must get a unique frontend request ID",
  );
  assert.match(
    mainSource,
    /cancelGithubBeforePrompt\(\{\s*requestId: githubRequestId,[\s\S]*?invoke<boolean>\("github_login_cancel_start", \{ requestId \}\)/,
    "the pending-prompt cancel path must carry that frontend request ID",
  );
});

test("finishes a pending Claude or Codex cancellation with its exact session", () => {
  assert.match(
    mainSource,
    /activeAccountRequestId = provider === "github" \? null : crypto\.randomUUID\(\)[\s\S]*?invoke<LoginPrompt>\("start_login", \{ provider, requestId \}\)/,
    "each Claude or Codex start must carry its frontend request ID",
  );
  assert.match(
    mainSource,
    /invoke<CancelOutcome>\("cancel_login_start", \{\s*requestId: accountRequestId,\s*\}\)[\s\S]*?cancelledWithCleanupWarning\(outcome\)[\s\S]*?if \(!cancelled\)[\s\S]*?Promise\.allSettled\(\[start\]\)/,
    "a cancellation recorded before worker startup must close immediately without waiting for a prompt",
  );
});

test("reserves every login request before start and releases it in finally", () => {
  assert.match(
    mainSource,
    /function addAccountButton[\s\S]*?await invoke\("reserve_login_start", \{ provider, requestId \}\);[\s\S]*?invoke<LoginPrompt>\("start_login", \{ provider, requestId \}\)[\s\S]*?finally \{[\s\S]*?invoke\("release_login_start", \{ provider, requestId \}\)/,
    "Claude and Codex must await an explicit reservation before starting and release it on every exit",
  );
  assert.match(
    mainSource,
    /function githubAddButton[\s\S]*?await invoke\("reserve_login_start", \{ provider: "github", requestId \}\);[\s\S]*?invoke<GithubLoginPrompt>\("github_login_start"[\s\S]*?finally \{[\s\S]*?invoke\("release_login_start", \{ provider: "github", requestId \}\)/,
    "GitHub must use the same explicit reservation lifecycle",
  );
});

test("uses the executable lifecycle helpers for cancellation decisions", () => {
  assert.match(
    mainSource,
    /cancelExactSession,[\s\S]*?cancelGithubBeforePrompt,[\s\S]*?cancelledWithCleanupWarning as cancelOutcomeCloses,[\s\S]*?decideFailedLogin,/,
    "the UI owner must import the runtime-tested lifecycle helpers",
  );
  assert.match(
    mainSource,
    /cancelGithubBeforePrompt\(\{[\s\S]*?githubCompletion = result\.completion;[\s\S]*?if \(completionWon\)[\s\S]*?ghAdded/,
    "the UI must report the exact completion drained by the helper",
  );
});

test("connects generic cleanup warnings to the runtime-tested cancellation result", () => {
  assert.match(
    mainSource,
    /function cancelledWithCleanupWarning\(outcome: CancelOutcome\): boolean \{\s*return cancelOutcomeCloses\(outcome, \(message\) => toast\(message, true\)\);\s*\}/,
    "cleanup-only failures must be rendered as warnings without rejecting cancellation",
  );
});

test("retains exact cancel controls when a Claude or Codex start leaves a live session", () => {
  assert.match(
    mainSource,
    /function addAccountButton[\s\S]*?decideFailedLogin\(\{[\s\S]*?login_session_for_request[\s\S]*?decision\.action === "retain"[\s\S]*?retainFailedLoginForCancel\(message, decision\.sessionId, attempt\);/,
    "a matching preserved session must keep the panel while an ordinary cleaned failure closes it",
  );
  assert.match(
    mainSource,
    /function retainFailedLoginForCancel[\s\S]*?loginSessionId = sessionId;[\s\S]*?cancelBtn\.addEventListener\("click", \(\) => void cancelActiveLogin\(attempt\)\)[\s\S]*?mountLoginPanel\(panel, attempt\);/,
    "the retained panel must keep the exact session ID and an enabled retry-cancel action",
  );
});

test("retains GitHub prompt failures and drains completion through the exact recovered waiter", () => {
  assert.match(
    mainSource,
    /function githubAddButton[\s\S]*?decideFailedLogin\(\{[\s\S]*?github_login_session_for_request[\s\S]*?decision\.action === "retain"[\s\S]*?retainFailedLoginForCancel\(message, decision\.sessionId, attempt\);\s*watchGithubLogin\(decision\.sessionId, requestId, attempt\);/,
    "GitHub start failures with a surviving process must keep cancel controls and start an exact waiter",
  );
  assert.match(
    mainSource,
    /function watchGithubLogin\(\s*sessionId: string,\s*requestId: string,\s*attempt: number,\s*\): Promise<string> \{\s*const wait = invoke<string>\("github_login_wait", \{ sessionId \}\);[\s\S]*?toast\(t\("ghAdded", \{ login \}\)\)[\s\S]*?finishLogin\(attempt\);/,
    "a completion that wins after prompt failure must still be consumed and reported",
  );
  assert.match(
    mainSource,
    /function watchGithubLogin[\s\S]*?catch \(error\)[\s\S]*?decideFailedLogin\(\{[\s\S]*?github_login_session_for_request[\s\S]*?decision\.sessionId === sessionId[\s\S]*?retainFailedLoginForCancel\(message, sessionId, attempt\);/,
    "a waiter termination failure must preserve the exact session and retry-cancel controls",
  );
  assert.match(
    mainSource,
    /if \(completionWon\) \{\s*if \(provider === "github" && !githubCompletion && !githubWait && sessionId\) \{\s*const wait = invoke<string>\("github_login_wait", \{ sessionId \}\);/,
    "a retry after a retained waiter failure must start a fresh exact waiter to drain completion",
  );
});

test("blocks account switches while an account login is active", () => {
  assert.match(
    mainSource,
    /const doSwitch = async[\s\S]*?if \(loginOpen\)[\s\S]*?return;/,
    "normal account cards must not switch underneath an active login",
  );
  assert.match(
    mainSource,
    /const interactionPanelOpen = loginOpen \|\| starPromptOpen;[\s\S]*?if \(locked\) \{\s*if \(!interactionPanelOpen\) \{[\s\S]*?\.card\.switchable/,
    "click-through account cards must not expose native switch actions behind an interaction panel",
  );
});

test("keeps a blocked shutdown visible and above a pending restart notice", () => {
  assert.match(
    mainSource,
    /function showShutdownStatus[\s\S]*?if \(shutdownState === "blocked" && state !== "blocked"\) return;[\s\S]*?shutdownStatus\.classList\.toggle\("shutdown-blocked", state === "blocked"\);/,
    "a process-termination failure must remain the dominant shutdown state",
  );
  assert.match(
    mainSource,
    /if \(shutdownState !== "idle"\) buffer\.prepend\(shutdownStatus\);[\s\S]*?app\.replaceChildren\(buffer\);/,
    "normal rerenders must retain the persistent shutdown status",
  );
  assert.match(
    mainSource,
    /listen<string>\("update-restarting"[\s\S]*?showShutdownStatus\("restarting"[\s\S]*?listen<string>\("shutdown-blocked"[\s\S]*?showShutdownStatus\("blocked", event\.payload\);/,
    "the backend termination failure must replace the earlier restart notice",
  );
  assert.match(
    stylesSource,
    /\.shutdown-status\.shutdown-blocked \{[\s\S]*?background: rgba\(127, 29, 29, 0\.96\);[\s\S]*?font-weight: 700;/,
    "the blocked state must be visibly distinct even when the widget background is transparent",
  );
});
