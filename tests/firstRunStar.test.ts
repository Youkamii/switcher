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

test("wires a star-above-label dialog without starring on mount", () => {
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
  assert.match(stylesSource, /\.star-prompt-action \{[\s\S]*?flex-direction: column;/);
  assert.match(stylesSource, /\.star-prompt-close \{[\s\S]*?position: absolute;/);
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
  assert.match(mainSource, /const width = loginOpen \|\| starPromptOpen\s*\? 360/);
});
