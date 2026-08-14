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

test("wires a compact inline prompt without starring on mount", () => {
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
  const titlebarRules = stylesSource.match(
    /body\.star-prompt-open \.titlebar \{[\s\S]*?\n\}/,
  )?.[0];
  assert.ok(promptRules);
  assert.ok(actionRules);
  assert.ok(closeRules);
  assert.ok(shellRules);
  assert.ok(titlebarRules);
  assert.match(promptRules, /display: flex;/);
  assert.match(promptRules, /gap: 6px;/);
  assert.match(promptRules, /padding: 20px 0;/);
  assert.match(actionRules, /height: 32px;/);
  assert.match(closeRules, /width: 32px;/);
  assert.match(closeRules, /height: 32px;/);
  assert.match(shellRules, /background: rgba\(17, 17, 24, 0\.98\);/);
  assert.match(titlebarRules, /display: none;/);
  assert.doesNotMatch(actionRules, /flex-direction: column/);
  assert.doesNotMatch(promptRules, /gradient|box-shadow/);
  assert.doesNotMatch(actionRules, /gradient|box-shadow/);
  assert.doesNotMatch(closeRules, /position: absolute/);
  assert.doesNotMatch(stylesSource, /\.star-prompt::before|\.star-prompt::after/);
});

test("blocks early renders and keeps the prompt clickable in compact modes", () => {
  assert.match(
    mainSource,
    /async function render[\s\S]*?if \(startupState !== "ready"\)[\s\S]*?mountGithubStarPrompt\(\);[\s\S]*?return;/,
  );
  assert.match(
    mainSource,
    /invoke<GithubStarPromptChoice \| null>\([\s\S]*?"get_github_star_prompt_choice"/,
  );
  assert.match(
    mainSource,
    /const interactionPanelOpen = loginOpen \|\| starPromptOpen;[\s\S]*?if \(interactionPanelOpen\)[\s\S]*?regions\.push\(\{ rect, action: null \}\)/,
  );
  assert.match(
    mainSource,
    /const width = loginOpen\s*\? 360\s*: starPromptOpen\s*\? 240/,
  );
});
