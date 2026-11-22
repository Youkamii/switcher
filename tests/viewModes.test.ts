import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const cssSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const i18nSource = readFileSync(new URL("../src/i18n.ts", import.meta.url), "utf8").replace(/\r\n/g, "\n");

/// #183 — 컴팩트(옛 2번) 폐지. 순환은 1 일반 → 2 미니멀 → 3 엣지.
test("the view cycle has three modes and no compact mode", () => {
  assert.match(mainSource, /const VIEW_CYCLE = \["normal", "minimal", "edge"\] as const;/);
  // 컴팩트 전용 렌더·분기·폭이 남아 있지 않다
  assert.doesNotMatch(mainSource, /renderGithubCompact|compactLabel\(|mode === "compact"|viewMode === "compact"/);
  assert.doesNotMatch(mainSource, /\? 280/);
  assert.doesNotMatch(cssSource, /\.compact-head|\.compact-toggle|\.c-reset|\.c-stale/);
  // 모드 탭은 세 칸
  assert.match(cssSource, /\.mode-tabs \{[^}]*grid-template-columns: repeat\(3,minmax\(0,1fr\)\);/);
});

test("stored and forced compact/locked modes migrate to minimal", () => {
  const migrate = mainSource.match(/function migrateViewMode\([\s\S]*?\n}\n/)?.[0];
  assert.ok(migrate, "migrateViewMode must exist");
  assert.match(migrate, /value === "locked" \|\| value === "compact" \? "minimal" : value/);
  // 저장값(localStorage)과 SWITCHER_VIEW 훅 둘 다 같은 이관을 거친다
  assert.match(mainSource, /const stored = migrateViewMode\(localStorage\.getItem\("switcher\.viewmode"\)\);/);
  assert.match(mainSource, /mode = migrateViewMode\(mode\) as string \| null;/);
  assert.match(mainSource, /localStorage\.getItem\("switcher\.locked"\) === "1" \? "minimal" : "normal"/);
});

test("mode numbers come from the cycle order in every place that shows them", () => {
  const name = mainSource.match(/function viewModeName\([\s\S]*?\n}\n/)?.[0];
  assert.ok(name, "viewModeName must exist");
  assert.match(name, /VIEW_CYCLE\.indexOf\(mode\) \+ 1/);
  // 타이틀바 버튼·일반 모드의 모드 탭·엣지 판의 모드 버튼
  assert.match(mainSource, /lockBtn\.textContent = viewModeName\(displayMode\);/);
  assert.match(mainSource, /button\.textContent = viewModeName\(value\);/);
  assert.match(mainSource, /mode\.textContent = viewModeName\("edge"\);/);
  assert.doesNotMatch(mainSource, /"4 엣지"|"4 Edge"|"2 컴팩트"|"2 Compact"/);
  // 툴팁 6개 언어에 옛 4단계 표기가 남아 있지 않다
  const tips = i18nSource.match(/^\s*typeTooltip: .*$/gm) ?? [];
  assert.equal(tips.length, 6);
  for (const tip of tips) assert.doesNotMatch(tip, /4|컴팩트|compact|コンパクト|紧凑|精簡|कॉम्पैक्ट/i);
});
