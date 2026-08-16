import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  completeFirstRunStarChoice,
  firstRunStartupState,
  STAR_CTA_LABEL,
} from "../src/firstRunStar.ts";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const rustSource = readFileSync(
  new URL("../src-tauri/src/lib.rs", import.meta.url),
  "utf8",
);

test("shows the first-run gate only until a stable choice exists", () => {
  assert.equal(firstRunStartupState(null), "star-prompt");
  assert.equal(firstRunStartupState("star"), "ready");
  assert.equal(firstRunStartupState("dismissed"), "ready");
  assert.equal(STAR_CTA_LABEL, "Star me on GitHub");
});

test("dismiss saves the choice and reveals the app without starring", async () => {
  const calls: string[] = [];
  await completeFirstRunStarChoice("dismissed", {
    persist: async (choice) => {
      calls.push(`persist:${choice}`);
    },
    revealApp: () => calls.push("reveal"),
    starRepository: async () => {
      calls.push("star");
    },
    reportError: () => calls.push("error"),
  });

  assert.deepEqual(calls, ["persist:dismissed", "reveal"]);
});

test("star saves the choice, reveals the app, then starts the request", async () => {
  const calls: string[] = [];
  await completeFirstRunStarChoice("star", {
    persist: async (choice) => {
      calls.push(`persist:${choice}`);
    },
    revealApp: () => calls.push("reveal"),
    starRepository: async () => {
      calls.push("star");
    },
    reportError: () => calls.push("error"),
  });

  assert.deepEqual(calls, ["persist:star", "reveal", "star"]);
});

test("a settings failure never turns the optional prompt into a startup block", async () => {
  const calls: string[] = [];
  await completeFirstRunStarChoice("star", {
    persist: async () => {
      calls.push("persist");
      throw new Error("disk unavailable");
    },
    revealApp: () => calls.push("reveal"),
    starRepository: async () => {
      calls.push("star");
    },
    reportError: () => calls.push("error"),
  });

  assert.deepEqual(calls, ["persist", "error", "reveal", "star"]);
  assert.match(
    rustSource,
    /fn get_github_star_prompt_choice\(\) -> Result<Option<String>, String> \{\s*let env = Env::real\(\)\?;/,
  );
});

test("wires a compact overlay without starring on mount", () => {
  assert.match(mainSource, /starPromptHost\.setAttribute\("role", "dialog"\)/);
  assert.match(mainSource, /close\.textContent = "×"/);
  assert.match(mainSource, /chooseGithubStarPrompt\("dismissed"\)/);
  assert.match(mainSource, /star\.textContent = "★"/);
  assert.match(mainSource, /label\.textContent = STAR_CTA_LABEL/);
  assert.match(mainSource, /chooseGithubStarPrompt\("star"\)/);

  const mountSource = mainSource.match(
    /function mountGithubStarPrompt\(\)[\s\S]*?\n\}/,
  )?.[0];
  assert.ok(mountSource);
  assert.doesNotMatch(mountSource, /github_star_repository/);
  assert.match(mountSource, /starPromptHost\.append\(action, close\)/);
  assert.match(mountSource, /shell\.appendChild\(starPromptHost\)/);
  assert.doesNotMatch(
    mountSource,
    /OPEN SOURCE|star-prompt-title|star-prompt-copy|star-prompt-eyebrow/,
  );

  const promptRules = stylesSource.match(/\.star-prompt \{[\s\S]*?\n\}/)?.[0];
  const actionRules = stylesSource.match(/\.star-prompt-action \{[\s\S]*?\n\}/)?.[0];
  const closeRules = stylesSource.match(/\.star-prompt-close \{[\s\S]*?\n\}/)?.[0];
  const shellRules = stylesSource.match(
    /body\.star-prompt-open \.shell \{[\s\S]*?\n\}/,
  )?.[0];
  const bodyRules = stylesSource.match(
    /body\.star-prompt-open \{[\s\S]*?\n\}/,
  )?.[0];
  assert.ok(promptRules);
  assert.ok(actionRules);
  assert.ok(closeRules);
  assert.ok(shellRules);
  assert.ok(bodyRules);
  assert.match(promptRules, /position: absolute;/);
  assert.match(promptRules, /inset: 0;/);
  assert.match(promptRules, /display: flex;/);
  assert.match(promptRules, /align-items: center;/);
  assert.match(promptRules, /justify-content: center;/);
  assert.match(promptRules, /background: rgba\(12, 12, 18, 0\.56\);/);
  assert.match(promptRules, /backdrop-filter: blur\(2px\);/);
  assert.match(actionRules, /height: 32px;/);
  assert.doesNotMatch(actionRules, /position: absolute/);
  assert.match(closeRules, /width: 32px;/);
  assert.match(closeRules, /height: 32px;/);
  assert.match(closeRules, /position: absolute;/);
  assert.match(closeRules, /top: calc\(50% \+ 22px\);/);
  assert.match(closeRules, /left: 50%;/);
  assert.match(closeRules, /transform: translateX\(-50%\);/);
  assert.match(shellRules, /position: relative;/);
  assert.match(bodyRules, /--bg-alpha: 1;/);
  assert.match(bodyRules, /--fg-alpha: 1;/);
  assert.match(bodyRules, /--bar-alpha: 1;/);
  assert.match(bodyRules, /--bg: rgb\(22, 22, 30\);/);
  assert.match(bodyRules, /--panel: rgb\(31, 31, 42\);/);
  assert.match(bodyRules, /--panel-2: rgb\(38, 38, 54\);/);
  assert.match(bodyRules, /--text: rgb\(230, 230, 239\);/);
  assert.match(bodyRules, /--muted: rgb\(139, 139, 158\);/);
  assert.doesNotMatch(actionRules, /flex-direction: column/);
  assert.doesNotMatch(promptRules, /gradient|box-shadow/);
  assert.doesNotMatch(actionRules, /gradient|box-shadow/);
  assert.doesNotMatch(stylesSource, /\.star-prompt::before|\.star-prompt::after/);
});

test("renders the default interface before the overlay and blocks background input", () => {
  assert.match(
    mainSource,
    /async function render[\s\S]*?if \(startupState === "checking"\) return;/,
  );
  assert.match(
    mainSource,
    /invoke<GithubStarPromptChoice \| null>\([\s\S]*?"get_github_star_prompt_choice"/,
  );
  assert.match(
    mainSource,
    /starPromptOpen = true;\s*applyViewMode\(\);\s*renderFirstRunBackdrop\(\);\s*mountGithubStarPrompt\(\);\s*void render\(\{ immediate: true \}\);/,
  );
  assert.match(
    mainSource,
    /function renderFirstRunBackdrop\(\)[\s\S]*?for \(const provider of PROVIDERS\)[\s\S]*?renderMonitor\(buffer\);[\s\S]*?app\.replaceChildren\(buffer\);/,
  );
  assert.match(
    mainSource,
    /const mode = starPromptOpen \? "normal" : viewMode;/,
  );
  assert.match(mainSource, /if \(!visibility\[key\] && !starPromptOpen\) continue;/);
  assert.match(mainSource, /if \(!monitorOn && !starPromptOpen\) continue;/);
  assert.match(
    mainSource,
    /if \(\(!monitorOn && !starPromptOpen\) \|\| monInflight\) return;/,
  );
  assert.match(mainSource, /titlebarEl\.inert = true;\s*app\.inert = true;/);
  assert.match(mainSource, /titlebarEl\.inert = false;\s*app\.inert = false;/);
  assert.match(mainSource, /const nativeLocked = viewMode !== "normal";/);
  assert.match(mainSource, /invoke\("set_click_through", \{ enabled: nativeLocked \}\)/);
  const hitRegionSource = mainSource.match(/function reportHitRegions\(\)[\s\S]*?\n\}/)?.[0];
  assert.ok(hitRegionSource);
  assert.match(
    hitRegionSource,
    /if \(locked \|\| starPromptOpen\)[\s\S]*?const interactionTarget = starPromptOpen \? starPromptHost : app;[\s\S]*?regions\.push\(\{ rect, action: null \}\)/,
  );
  assert.match(
    mainSource,
    /const width = loginOpen \|\| starPromptOpen\s*\? 360/,
  );
});
