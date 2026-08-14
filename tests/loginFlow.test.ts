import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const loginScrollRegion =
  /if \(loginOpen\) \{\s*const rect = visibleHitRect\(\s*app\.getBoundingClientRect\(\),\s*window\.innerWidth,\s*window\.innerHeight,?\s*\);\s*if \(rect\) \{\s*regions\.push\(\{ rect, action: null \}\);\s*hitElements\.push\(app\);/;

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
    /const width = loginOpen\s*\? 360\s*:/,
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

test("keeps the full scroll viewport interactive only while login is open", () => {
  assert.match(
    mainSource,
    /if \(locked\) \{[\s\S]*?if \(loginOpen\) \{\s*const rect = visibleHitRect\(\s*app\.getBoundingClientRect\(\)/,
    "the full main viewport must only become a native UI region in click-through mode during login",
  );
  assert.doesNotMatch(
    mainSource,
    /querySelectorAll<HTMLElement>\([^)]*#app[^)]*\)/,
    "the regular selector list must not make the whole app interactive after login closes",
  );
  assert.match(stylesSource, /main \{[\s\S]*?min-height: 0;[\s\S]*?overflow-y: auto;/);
});

test("guards login completion and serializes GitHub cancellation", () => {
  assert.match(
    mainSource,
    /function finishLogin\(attempt: number\)[\s\S]*?if \(!isCurrentLogin\(attempt\)\) return;/,
    "an older async login must not close the current panel",
  );
  assert.match(
    mainSource,
    /loginCancelingAttempt = attempt;[\s\S]*?invoke<boolean>\("github_login_cancel", \{ sessionId \}\)[\s\S]*?finishLogin\(attempt\);/,
    "GitHub cancellation must finish before another login can start",
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
    /watchGithubLogin\(prompt\.session_id, attempt\)/,
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
    /invoke<boolean>\("github_login_cancel_start", \{\s*requestId: githubRequestId,/,
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

test("treats an already completed exact login cancellation as idempotent", () => {
  assert.match(
    mainSource,
    /invoke<CancelOutcome>\("cancel_login", \{ sessionId \}\)[\s\S]*?completionWon = !cancelledWithCleanupWarning\(outcome\)[\s\S]*?if \(completionWon\)[\s\S]*?Promise\.allSettled\(\[accountWait\]\)[\s\S]*?reportLogin\(finished\.value\)/,
    "when completion wins, the completed account result must be reported instead of pretending cancellation won",
  );
  assert.match(
    mainSource,
    /completionWon = !\(await invoke<boolean>\("github_login_cancel", \{ sessionId \}\)\)[\s\S]*?if \(completionWon\)[\s\S]*?Promise\.allSettled\(\[githubWait\]\)[\s\S]*?ghAdded/,
    "GitHub completion must likewise be reported when it wins the cancellation race",
  );
});

test("shows isolated-cleanup warnings after verified cancellation and still closes once", () => {
  assert.match(
    mainSource,
    /function cancelledWithCleanupWarning\(outcome: CancelOutcome\): boolean \{\s*if \(outcome\.cleanup_error\) toast\(outcome\.cleanup_error, true\);\s*return outcome\.cancelled;\s*\}/,
    "cleanup-only failures must be rendered as warnings without rejecting cancellation",
  );
  assert.match(
    mainSource,
    /async function cancelActiveLogin[\s\S]*?invoke<CancelOutcome>\("cancel_login_start"[\s\S]*?cancelledWithCleanupWarning\(outcome\)[\s\S]*?finishLogin\(attempt\);/,
    "the pre-prompt path must report cleanup warnings and then close the login panel",
  );
  assert.match(
    mainSource,
    /invoke<CancelOutcome>\("cancel_login", \{ sessionId \}\)[\s\S]*?cancelledWithCleanupWarning\(outcome\)[\s\S]*?finishLogin\(attempt\);/,
    "the post-prompt path must report cleanup warnings and then close the login panel",
  );
});

test("finishes GitHub login when completion beats a pre-prompt cancel", () => {
  assert.match(
    mainSource,
    /github_login_cancel_start[\s\S]*?github_login_cancel[\s\S]*?if \(!cancelled\) \{\s*completionWon = true;\s*githubWait = invoke<string>\("github_login_wait", \{\s*sessionId: started\.value\.session_id,[\s\S]*?activeGithubWait = githubWait;/,
    "a completed pre-prompt GitHub session must be consumed instead of being left behind",
  );
});

test("retains exact cancel controls when a Claude or Codex start leaves a live session", () => {
  assert.match(
    mainSource,
    /invoke<string \| null>\("login_session_for_request", \{\s*requestId,\s*\}\)[\s\S]*?if \(sessionId\) \{\s*retainFailedLoginForCancel\(message, sessionId, attempt\);\s*return;\s*\}[\s\S]*?toast\(message, true\);\s*finishLogin\(attempt\);/,
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
    /invoke<string \| null>\("github_login_session_for_request", \{\s*requestId,\s*\}\)[\s\S]*?retainFailedLoginForCancel\(message, sessionId, attempt\);\s*watchGithubLogin\(sessionId, attempt\);\s*return;/,
    "GitHub start failures with a surviving process must keep cancel controls and start an exact waiter",
  );
  assert.match(
    mainSource,
    /function watchGithubLogin\(sessionId: string, attempt: number\): Promise<string> \{\s*const wait = invoke<string>\("github_login_wait", \{ sessionId \}\);[\s\S]*?toast\(t\("ghAdded", \{ login \}\)\)[\s\S]*?finishLogin\(attempt\);/,
    "a completion that wins after prompt failure must still be consumed and reported",
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
    /if \(locked\) \{\s*if \(!loginOpen\) \{[\s\S]*?\.card\.switchable/,
    "click-through account cards must not expose native switch actions during login",
  );
});
