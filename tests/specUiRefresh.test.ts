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
  assert.match(mainSource, /head\.append\(statusDot\(acc\.active\), name\);/, "GitHub cards get the same dot");
  // Type3(minimal)는 머리를 만들지 않는다 — compactCard의 머리가 !minimal 블록 안에서 닫히기 전에
  // dot이 붙어야 한다 (블록 닫힘 "\n  }\n"을 넘지 않는 tempered 패턴)
  const compact = mainSource.match(/function compactCard\([\s\S]*?\n}\n/)?.[0];
  assert.ok(compact, "compactCard must exist");
  assert.match(compact, /if \(!minimal\) \{(?:(?!\n  \}\n)[\s\S])*?statusDot\(profile\.active\)/);
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
  assert.match(icon, /width: 16px;/);
  assert.match(icon, /height: 16px;/);
  assert.match(icon, /shape-rendering: crispEdges;/);
  assert.match(icon, /opacity: var\(--fg-alpha\);/, "fades with the title text");
  assert.match(cssSource, /\.prov-icon\.prov-claude \{[^}]*fill: rgb\(var\(--accent-rgb\)\);/);
  assert.match(cssSource, /\.prov-icon\.prov-codex \{[^}]*fill: rgb\(var\(--accent-alt-rgb\)\);/);
  const compactIcon = cssSource.match(/\.compact-head \.prov-icon \{[^}]*\}/)?.[0];
  assert.ok(compactIcon, "compact icon rule must exist");
  assert.match(compactIcon, /width: 8px;/);
  assert.match(compactIcon, /height: 8px;/);
});

/// #103 — Type1 조작 버튼은 호버·포커스에만. 창 높이 보정, 커서 밑 생성 직후 클릭
/// 유예, 카드 이탈 시 삭제 확인 해제, 키보드 도달 경로가 한 벌로 있어야 한다.
test("card actions are hidden by default and shown on hover or keyboard focus in Type1", () => {
  const base = cssSource.match(/\.card-actions \{[^}]*\}/)?.[0];
  assert.ok(base, ".card-actions rule must exist");
  // display:none이면 Tab으로 카드→버튼 이동 순간 버튼이 포커스 불가가 돼 body로 떨어진다 (CDP 실측)
  // — 높이 0 + overflow hidden으로 접어 버튼은 포커스 가능한 채 보이지만 않게 한다
  assert.match(base, /display: flex;/);
  assert.match(base, /height: 0;/);
  assert.match(base, /overflow: hidden;/);
  assert.doesNotMatch(base, /display: none/);
  const expanded = /\{[^}]*height: auto;[^}]*overflow: visible;/;
  assert.match(cssSource, new RegExp("#app:not\\(\\.locked\\) \\.card:hover \\.card-actions " + expanded.source));
  // 키보드 경로는 :focus-within이 아니라 :has(:focus-visible) — 마우스로 누른 버튼의 포커스로
  // 버튼 줄이 눌러붙지 않게. 호버 규칙과 다른 규칙이어야 :has 미지원 엔진에서 호버가 산다
  assert.match(cssSource, new RegExp("#app:not\\(\\.locked\\) \\.card:has\\(:focus-visible\\) \\.card-actions " + expanded.source));
  // :has는 자손만 — Tab이 카드 자체에 멈춘 순간도 버튼 줄이 보인다
  assert.match(cssSource, new RegExp("#app:not\\(\\.locked\\) \\.card:focus-visible \\.card-actions " + expanded.source));
  assert.doesNotMatch(cssSource, /\.card:focus-within/);
  // 전환 후보 카드의 호버 신호는 테마 accent를 따른다 (색감 6종 이후 고정색 금지)
  const hover = cssSource.match(/#app:not\(\.locked\) \.card\.switchable:hover \{[\s\S]*?\}/)?.[0];
  assert.ok(hover, "switchable hover rule must exist");
  assert.match(hover, /rgba\(var\(--accent-rgb\), calc\(0\.75 \* var\(--fg-alpha\)\)\)/);
  assert.doesNotMatch(hover, /167, 139, 250/);
  // 고정 모드(Type2/3)에는 .card-actions가 렌더되지 않고 기본값도 none — 따로 숨김 규칙이 없어야 한다
  assert.doesNotMatch(cssSource, /#app\.locked \.card-actions/);
});

test("hoverActions refits the window, grants a click grace after appearing, and disarms on leave", () => {
  const fn = mainSource.match(/function hoverActions\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn, "hoverActions must exist");
  assert.match(mainSource, /const ACTION_GRACE_MS = 250;/);
  // 유예는 마우스가 버튼 줄 높이로 들어온 진입에만, 터치·펜은 제외 (review)
  const enter = fn.match(/card\.addEventListener\("pointerenter", \(event\) => \{[\s\S]*?\n  \}\);/)?.[0];
  assert.ok(enter, "pointerenter handler must exist");
  assert.match(enter, /shownAt = 0;/);
  assert.match(enter, /if \(event\.pointerType !== "mouse"\) return;/);
  assert.match(enter, /rect\.height > 0 && event\.clientY >= rect\.top\) shownAt = Date\.now\(\);/);
  assert.match(enter, /refit\(\);/);
  const leave = fn.match(/card\.addEventListener\("pointerleave", \(event\) => \{[\s\S]*?\n  \}\);/)?.[0];
  assert.ok(leave, "pointerleave handler must exist");
  assert.match(leave, /if \(event\.pointerType === "mouse"\) disarm\?\.\(\);/);
  assert.match(leave, /refit\(\);/);
  assert.match(fn, /card\.addEventListener\("focusin", refit\);/);
  assert.match(fn, /card\.addEventListener\("focusout", \(\) => \{[^}]*disarm\?\.\(\);/);
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
  // #178의 해제 경로(바깥 클릭·blur·3초)는 tests/hiddenWebviewTimers.test.ts가 지킨다
  const github = mainSource.match(/function githubCard\([\s\S]*?\n}\n/)?.[0];
  assert.ok(github, "githubCard must exist");
  assert.match(github, /if \(!compact\) \{(?:(?!\n    \}\n)[\s\S])*?card\.tabIndex = 0;\s*const justShown = hoverActions\(card\);/);
  assert.match(github, /switchBtn\.addEventListener\("click", async \(\) => \{\s*if \(justShown\(\)\) return;/);
  // 컴팩트 카드·Type4 묶음에는 호버 버튼이 없으므로 hoverActions도 없다
  const compact = mainSource.match(/function compactCard\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.doesNotMatch(compact, /hoverActions\(/);
  const edge = mainSource.match(/function edgeAccount\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.doesNotMatch(edge, /hoverActions\(/);
});

/// #103 리뷰 반영 — 호버 버튼이 생기며 드러난 주변 결함
test("rename submission keeps focus (readOnly, not disabled) so the card does not collapse", () => {
  const rename = mainSource.match(/const submit = async \(\) => \{[\s\S]*?\n    \};\n/)?.[0];
  assert.ok(rename, "rename submit must exist");
  assert.match(rename, /input\.readOnly = true;/);
  assert.match(rename, /input\.readOnly = false;/);
  assert.doesNotMatch(rename, /input\.disabled/);
});

test("optimistic switch moves the status-dot tooltip with the active class", () => {
  const fn = mainSource.match(/function markActiveOptimistic\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn, "markActiveOptimistic must exist");
  assert.match(fn, /\.status-dot"\)\) dot\.removeAttribute\("title"\);/);
  assert.match(fn, /if \(dot\) dot\.title = t\("activeDot"\);/);
});

test("smooth render restores keyboard focus to the same card slot", () => {
  assert.match(
    mainSource,
    /const focusedCard = document\.activeElement\?\.closest\("\.card"\) \?\? null;[\s\S]*?app\.replaceChildren\(buffer\);[\s\S]*?commitEdgeGauge\(card\)\);\s*if \(focusedIdx >= 0\) \{\s*app\.querySelectorAll<HTMLElement>\("\.card"\)\[focusedIdx\]\?\.focus\(\{ preventScroll: true \}\);/,
  );
});

test("a window resting on the work-area bottom keeps its bottom edge when content height changes", () => {
  const fit = mainSource.match(/async function fitWindowToContent\(\) \{[\s\S]*?\n}\n/)?.[0];
  assert.ok(fit, "fitWindowToContent must exist");
  assert.match(mainSource, /const BOTTOM_ANCHOR_TOLERANCE = 4;/);
  assert.match(fit, /anchorBottom = workBottom;/);
  assert.match(fit, /const y = anchorBottom !== null \? anchorBottom - newSize\.height : topY;/);
  // Type4는 벽 스냅이 맡으므로 제외
  assert.match(fit, /!edgeActive && currentWorkArea/);
});
