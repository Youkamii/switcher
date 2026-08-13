import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("press feedback never moves or shrinks a button away from the pointer", () => {
  const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
  const pressingRule = css.match(/\.tb-actions button\.pressing\s*\{([^}]*)\}/);

  assert.ok(pressingRule, "pressing rule should exist");
  assert.doesNotMatch(pressingRule[1], /\btransform\s*:/);
});
