import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

/// 맥 비활성 웹뷰는 타이머·rAF가 멈출 수 있다 (CLAUDE.md macOS 절) — 필수 동작은
/// 사용자 동작이나 러스트 신호로도 끝나야 한다 (#178)
test("delete confirmation disarms on outside interaction, not only on a timer", () => {
  const block = mainSource.match(/const deleteBtn = document\.createElement\("button"\);[\s\S]*?actions\.appendChild\(deleteBtn\);/)?.[0];
  assert.ok(block, "delete button block must exist");
  assert.match(block, /document\.addEventListener\("pointerdown", disarmOutside, true\)/);
  assert.match(block, /window\.addEventListener\("blur", disarm\)/);
  assert.match(block, /disarmTimer = window\.setTimeout\(disarm, 3000\)/);
});

test("brightness sliders send the final value on change, not only after the debounce", () => {
  const changes = mainSource.match(/slider\.addEventListener\("change", send\);/g) ?? [];
  assert.equal(changes.length, 2, "both the Type1/compact and the Type4 sliders flush on change");
  const debounces = mainSource.match(/debounce = window\.setTimeout\(send, 250\);/g) ?? [];
  assert.equal(debounces.length, 2, "drag debounce stays at 250ms in both");
});

test("Type4 animation state machine unsticks from the next Rust hover signal", () => {
  assert.match(
    mainSource,
    /if \(edgeAnimating && Date\.now\(\) - edgeAnimStartedAt > EDGE_ANIM_MAX_MS\) edgeAnimationDone\(\);/,
  );
  assert.match(mainSource, /edgeAnimStartedAt = Date\.now\(\);/);
});
