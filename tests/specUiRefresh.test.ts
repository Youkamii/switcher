import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8").replace(/\r\n/g, "\n");
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

// #179 supersedes the hover-only interaction. Geometry is exercised in review-layout.mjs.
test("card actions stay reachable without hover and do not need a click grace", () => {
  const base = cssSource.match(/\.card-actions \{[^}]*\}/)?.[0];
  assert.ok(base);
  assert.match(base, /display: flex;/);
  assert.doesNotMatch(base, /height: 0;|display: none;/);
  assert.doesNotMatch(mainSource, /ACTION_GRACE_MS|justShown|function hoverActions/);
});

test("delete confirmation disarms on leaving the card, without resizing on hover", () => {
  const fn = mainSource.match(/function bindActionDisarm\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn);
  assert.match(fn, /pointerleave/);
  assert.match(fn, /focusout/);
  assert.match(fn, /card\.contains\(event\.relatedTarget\)/);
  assert.doesNotMatch(fn, /fitHeight|setTimeout/);
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

/// #103 맥 — 비활성 패널은 WKWebView :hover를 받지 못하므로(WebKit WebViewImpl.mm
/// trackingAreaOptions: NSTrackingActiveInKeyWindow) 러스트 커서 폴링으로 호버를 만든다.
test("Type1 hover on macOS is driven by Rust cursor polling (hover-poll), mirroring :hover", () => {
  const apply = mainSource.match(/function applyViewMode\(\) \{[\s\S]*?\n}\n/)?.[0];
  assert.ok(apply, "applyViewMode must exist");
  assert.match(apply, /const hoverPoll = viewMode === "normal" && !starPromptOpen;/);
  assert.match(apply, /invoke\("set_hover_poll", \{ enabled: hoverPoll \}\)/);
  assert.match(apply, /if \(!hoverPoll\) applyPolledHover\(null, 0, 0\);/);
  // Type1에서는 모든 카드 영역을 보고한다 (투과 없음 → action null)
  const report = mainSource.match(/function reportHitRegions\(\) \{[\s\S]*?\n}\n/)?.[0];
  assert.ok(report, "reportHitRegions must exist");
  assert.match(report, /\} else if \(!interactionPanelOpen\) \{\s*(?:\/\/[^\n]*\n\s*)*app\.querySelectorAll<HTMLElement>\("\.card"\)\.forEach\(\(el\) => pushVisible\(el, null\)\);/);
  assert.match(report, /syncPolledHover\(\);/);
  // 클래스를 먼저 붙이고 합성 pointerenter — hoverActions의 유예 판정이 펼쳐진 버튼 줄을 잰다
  const fn = mainSource.match(/function applyPolledHover\([\s\S]*?\n}\n/)?.[0];
  assert.ok(fn, "applyPolledHover must exist");
  assert.match(fn, /el\.classList\.add\("hit-hover"\);\s*el\.dispatchEvent\(new PointerEvent\("pointerenter", \{ pointerType: "mouse", clientX: x, clientY: y \}\)\);/);
  assert.match(fn, /prev\.classList\.remove\("hit-hover"\);[\s\S]*?new PointerEvent\("pointerleave", \{ pointerType: "mouse"/);
  assert.match(mainSource, /listen<\{ idx: number; x\?: number; y\?: number \}>\("hover-poll"/);
  // Hover remains a colour cue only; it no longer reveals an action row.
  assert.doesNotMatch(cssSource, /\.card\.hit-hover \.card-actions/);
  assert.match(cssSource, /#app:not\(\.locked\) \.card\.switchable\.hit-hover \{[^}]*rgba\(var\(--accent-rgb\)/);
});
