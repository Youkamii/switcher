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

/// #104 — 프로바이더 픽셀 아이콘. Type1 섹션 제목(계정 유무 모두)과 Type2 컴팩트
/// 머리글 앞에 붙고, 색은 Type3 스트라이프와 같은 테마 변수를 쓴다.
test("provider pixel icon is prepended to Type1 titles and Type2 compact heads", () => {
  const prepends = mainSource.match(/heading\.prepend\(providerIcon\(provider(?:\.id)?\)\);/g) ?? [];
  assert.equal(prepends.length, 2, "renderProvider and the empty-state section both get the icon");
  assert.match(mainSource, /head\.append\(providerIcon\(provider\), name\);/, "compact head gets the icon");
  // SYSTEM 제목(.mon-title)과 Type4에는 붙지 않는다
  assert.doesNotMatch(mainSource, /mon-title[\s\S]{0,400}providerIcon\(/);
  const edge = mainSource.match(/async function renderProviderEdge\([\s\S]*?\n}\n/)?.[0];
  assert.ok(edge, "renderProviderEdge must exist");
  assert.doesNotMatch(edge, /providerIcon\(/);
});

test("provider icon is one path per icon, coloured by theme variables, sized to whole cells", () => {
  const fn = mainSource.match(/function providerIcon\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn, "providerIcon must exist");
  assert.match(fn, /createElementNS\(SVG_NS, "path"\)/);
  assert.doesNotMatch(fn, /createElementNS\(SVG_NS, "rect"\)/);
  assert.doesNotMatch(fn, /setAttribute\("fill"/, "colour comes from CSS, not the element");
  assert.match(fn, /`prov-icon prov-\$\{provider\}`/);
  // 격자는 8줄 × 8칸
  const grids = mainSource.match(/const PROVIDER_ICONS[\s\S]*?\n};\n/)?.[0] ?? "";
  const rows = grids.match(/"[.x]{8}"/g) ?? [];
  assert.equal(rows.length, 16, "two 8x8 grids");
  const icon = cssSource.match(/\.prov-icon \{[^}]*\}/)?.[0];
  assert.ok(icon, ".prov-icon rule must exist");
  assert.match(icon, /width: 16px;\s*height: 16px;/);
  assert.match(icon, /shape-rendering: crispEdges;/);
  assert.match(icon, /opacity: var\(--fg-alpha\);/, "fades with the title text");
  assert.match(cssSource, /\.prov-icon\.prov-claude \{\s*fill: rgb\(var\(--accent-rgb\)\);/);
  assert.match(cssSource, /\.prov-icon\.prov-codex \{\s*fill: rgb\(var\(--accent-alt-rgb\)\);/);
  assert.match(cssSource, /\.compact-head \.prov-icon \{\s*width: 8px;\s*height: 8px;/);
});

/// #103 — Type1 조작 버튼은 호버·포커스에만. 창 높이 보정, 커서 밑 생성 직후 클릭
/// 유예, 카드 이탈 시 삭제 확인 해제, 키보드 도달 경로가 한 벌로 있어야 한다.
test("card actions are hidden by default and shown on hover or keyboard focus in Type1", () => {
  const base = cssSource.match(/\.card-actions \{[^}]*\}/)?.[0];
  assert.ok(base, ".card-actions rule must exist");
  assert.match(base, /display: none;/);
  assert.match(
    cssSource,
    /#app:not\(\.locked\) \.card:hover \.card-actions,\s*#app:not\(\.locked\) \.card:focus-within \.card-actions \{\s*display: flex;/,
  );
  // 전환 후보 카드의 호버 신호는 테마 accent를 따른다 (색감 6종 이후 고정색 금지)
  const hover = cssSource.match(/#app:not\(\.locked\) \.card\.switchable:hover,[\s\S]*?\}/)?.[0];
  assert.ok(hover, "switchable hover rule must exist");
  assert.match(hover, /rgba\(var\(--accent-rgb\), calc\(0\.75 \* var\(--fg-alpha\)\)\)/);
  assert.doesNotMatch(hover, /167, 139, 250/);
  // 고정 모드(Type2/3)의 통째 숨김은 그대로
  assert.match(cssSource, /#app\.locked \.card-actions,/);
});

test("hoverActions refits the window, grants a click grace after appearing, and disarms on leave", () => {
  const fn = mainSource.match(/function hoverActions\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn, "hoverActions must exist");
  assert.match(mainSource, /const ACTION_GRACE_MS = 250;/);
  assert.match(fn, /card\.addEventListener\("pointerenter", \(\) => \{\s*shownAt = Date\.now\(\);\s*refit\(\);/);
  assert.match(fn, /card\.addEventListener\("pointerleave", \(\) => \{\s*disarm\?\.\(\);\s*refit\(\);/);
  assert.match(fn, /card\.addEventListener\("focusin", refit\);/);
  assert.match(fn, /card\.addEventListener\("focusout", \(\) => \{\s*disarm\?\.\(\);/);
  assert.match(fn, /return \(\) => Date\.now\(\) - shownAt < ACTION_GRACE_MS;/);
  assert.match(fn, /if \(!app\.classList\.contains\("locked"\)\) fitHeight\(\);/, "no refit in widget modes");
});

test("profile and GitHub cards are keyboard-reachable and guard switch/delete clicks right after appearing", () => {
  const profile = mainSource.match(/function profileCard\([\s\S]*?\n}\n/)?.[0];
  assert.ok(profile, "profileCard must exist");
  assert.match(profile, /card\.tabIndex = 0;/);
  assert.match(profile, /const justShown = hoverActions\(card, \(\) => disarm\(\)\);/);
  assert.match(profile, /switchBtn\.addEventListener\("click", \(\) => \{\s*if \(justShown\(\)\) return;\s*void doSwitch\(switchBtn\);/);
  assert.match(profile, /deleteBtn\.addEventListener\("click", async \(\) => \{[\s\S]*?if \(justShown\(\)\) return;\s*if \(!armed\) \{/);
  // #178의 해제 경로(바깥 클릭·blur·3초)는 그대로 남는다
  assert.match(profile, /document\.addEventListener\("pointerdown", disarmOutside, true\);/);
  assert.match(profile, /disarmTimer = window\.setTimeout\(disarm, 3000\);/);
  const github = mainSource.match(/function githubCard\([\s\S]*?\n}\n/)?.[0];
  assert.ok(github, "githubCard must exist");
  assert.match(github, /if \(!compact\) \{[\s\S]*?card\.tabIndex = 0;\s*const justShown = hoverActions\(card\);/);
  assert.match(github, /switchBtn\.addEventListener\("click", async \(\) => \{\s*if \(justShown\(\)\) return;/);
  // 컴팩트 카드·Type4 묶음에는 호버 버튼이 없으므로 hoverActions도 없다
  const compact = mainSource.match(/function compactCard\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.doesNotMatch(compact, /hoverActions\(/);
  const edge = mainSource.match(/function edgeAccount\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.doesNotMatch(edge, /hoverActions\(/);
});
