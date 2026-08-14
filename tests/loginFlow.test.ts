import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

test("resizes the window after account login panels are mounted", () => {
  assert.match(
    mainSource,
    /slot\.appendChild\(\s*loginPanel\([\s\S]*?\)\s*,?\s*\);\s*fitHeight\(\);/,
    "Claude/Codex login panels must request a height fit after mounting",
  );
  assert.match(
    mainSource,
    /slot\.appendChild\(panel\);\s*fitHeight\(\);/,
    "GitHub login panels must request a height fit after mounting",
  );
});
