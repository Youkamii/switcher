import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const cssSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const i18nSource = readFileSync(new URL("../src/i18n.ts", import.meta.url), "utf8");

/// #102 — 활성 계정 dot. 글자·테두리가 --fg-alpha로 사라지는 골조 투명도에서도
/// 사용량 바와 같은 --bar-alpha로 남아야 한다.
test("status dot is attached to Type1 and Type2 card heads only", () => {
  const dots = mainSource.match(/head\.append\(statusDot\(profile\.active\), email\);/g) ?? [];
  assert.equal(dots.length, 2, "profileCard and compactCard both prepend the dot to the email");
  // Type3(minimal)는 머리를 만들지 않는다 — compactCard의 머리가 !minimal 안에 있어야 dot도 안 붙는다
  const compact = mainSource.match(/function compactCard\([\s\S]*?\n}\n/)?.[0];
  assert.ok(compact, "compactCard must exist");
  assert.match(compact, /if \(!minimal\) \{\s*head\.className = "card-head";[\s\S]*?statusDot\(profile\.active\)/);
  // Type4(edgeAccount)에는 붙지 않는다 — 이름 앞 ::before 표식이 그 역할
  const edge = mainSource.match(/function edgeAccount\([\s\S]*?\n}\n/)?.[0];
  assert.ok(edge, "edgeAccount must exist");
  assert.doesNotMatch(edge, /statusDot\(/);
});

test("status dot follows --bar-alpha, never --fg-alpha, and only the active dot has a tooltip", () => {
  const rule = cssSource.match(/\.status-dot \{[^}]*\}/)?.[0];
  const active = cssSource.match(/\.card\.active \.status-dot \{[^}]*\}/)?.[0];
  assert.ok(rule && active, "both dot rules must exist");
  for (const block of [rule, active]) {
    assert.match(block, /var\(--bar-alpha\)/);
    assert.doesNotMatch(block, /--fg-alpha/);
  }
  assert.match(mainSource, /if \(active\) dot\.title = t\("activeDot"\);/);
  assert.doesNotMatch(i18nSource, /inactiveDot/);
  const activeStrings = i18nSource.match(/^\s*activeDot:/gm) ?? [];
  assert.equal(activeStrings.length, 6, "activeDot tooltip exists in all 6 locales");
});

test("privacy blur targets the name only, so the dot stays sharp", () => {
  const blur = cssSource.match(/body\.privacy \.card-name,\s*body\.privacy \.hint\.warn \{[^}]*\}/)?.[0];
  assert.ok(blur, "privacy blur rule must exist");
  assert.doesNotMatch(cssSource, /body\.privacy[^{]*\.status-dot/);
});
