import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const mainSource = read("../src/main.ts");
const cssSource = read("../src/styles.css");
const libSource = read("../src-tauri/src/lib.rs");

/// #185 — 엣지(벽 붙임) 손잡이 높이. ☰ 드래그가 작업영역을 넘친 만큼(clamp로 되돌아온
/// 거리)을 손잡이 오프셋으로 흡수하고, 손잡이·게이지·판 원점·덩어리가 같은 변수를 따른다.
test("drag overshoot past the work area moves the edge handle instead of being lost", () => {
  // 버튼을 놓고 정착한 이동만 드래그로 본다 — 프로그램이 옮긴 창(clamp 직후·크기 변경)은
  // 이미 안에 있어 넘침이 0이다
  assert.match(mainSource, /void syncCurrentMonitor\(forceSync, true\);/);
  assert.match(mainSource, /async function syncCurrentMonitor\(force = false, fromDrag = false\)/);
  assert.match(mainSource, /snapToEdge\(fromDrag\)\.catch/);
  const snap = mainSource.match(/async function snapToEdge\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.match(snap, /^async function snapToEdge\(fromDrag = false\)/);
  // 넘침은 물리 px → 배율로 나눠 CSS px로, 아래가 양수
  assert.match(snap, /let overshoot = pos\.y - target\.y;[\s\S]*?if \(overshoot !== 0\) shiftEdgeTabY\(overshoot \/ scale\);/);
  // 위 벽은 OS가 창을 먼저 막는다(맥 메뉴 막대) — 커서가 ☰보다 위면 그만큼, 화면 맨 위면 끝까지
  assert.match(snap, /invoke<\[number, number\] \| null>\("pointer_position"\)/);
  assert.match(snap, /if \(cursor\[1\] <= 1\) overshoot = Number\.NEGATIVE_INFINITY;/);
  assert.match(libSource, /fn pointer_position\(app: tauri::AppHandle\) -> Option<\(f64, f64\)>/);
  assert.match(libSource, /\n\s+pointer_position,\n/, "the command must be registered");
});

test("the handle stays inside the window for any gauge height", () => {
  // 손잡이 키는 게이지 줄 수에 따라 변한다 — 범위는 현재 키의 절반 + 여유로 계산한다
  const bounds = mainSource.match(/function edgeTabBounds\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.match(bounds, /const min = edgeTabHeight \/ 2 \+ EDGE_TAB_MARGIN;/);
  assert.match(bounds, /Math\.max\(min, window\.innerHeight - edgeTabHeight \/ 2 - EDGE_TAB_MARGIN\)/);
  // 키가 바뀌면(계정·SYSTEM 줄 변화) 저장된 높이를 다시 가둔다
  assert.match(
    mainSource,
    /style\.setProperty\("--edge-tab-height", `\$\{geometry\.tabHeight\}px`\);\s*(?:\/\/[^\n]*\n\s*)*edgeTabHeight = geometry\.tabHeight;\s*applyEdgeTabY\(\);/,
  );
  const shift = mainSource.match(/function shiftEdgeTabY\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.match(shift, /localStorage\.setItem\("switcher\.edgetab", String\(edgeTabY\)\);/);
  // 창 높이가 바뀌면 다시 누른다 — 비활성 웹뷰 타이머가 아니라 resize 이벤트
  assert.match(mainSource, /window\.addEventListener\("resize", \(\) => \{\s*if \(edgeActive\) applyEdgeTabY\(\);/);
});

test("handle, gauge, panel origin and blobs all follow --edge-tab-y (default 50%)", () => {
  for (const sel of ["#edge-tab", "#edge-gauge"]) {
    const block = cssSource.match(new RegExp(sel + " \\{[^}]*\\}"))?.[0] ?? "";
    assert.match(block, /top: var\(--edge-tab-y, 50%\);/, `${sel} must read the variable`);
  }
  const body = cssSource.match(/#edge-body \{[^}]*\}/)?.[0] ?? "";
  assert.match(body, /transform-origin: right calc\(var\(--edge-tab-y, 50%\) - 12px\);/);
  assert.match(cssSource, /\.edge-blob:nth-child\(2\) \{[^}]*top: calc\(var\(--edge-tab-y, 50%\) - \(100% - 24px\) \* 0\.26 - 29px\);/);
  assert.match(cssSource, /\.edge-blob:nth-child\(3\) \{[^}]*top: calc\(var\(--edge-tab-y, 50%\) - 35px\);/);
  assert.match(cssSource, /\.edge-blob:nth-child\(4\) \{[^}]*top: calc\(var\(--edge-tab-y, 50%\) \+ \(100% - 24px\) \* 0\.24 - 31px\);/);
  // 숫자 확인: 기본 50%일 때 덩어리 위·아래의 윗변이 옛 식(24%·74%)과 같다
  for (const H of [317, 600, 901]) {
    assert.ok(Math.abs(12 + (H - 24) * 0.24 - 29 - (H / 2 - (H - 24) * 0.26 - 29)) < 1e-9);
    assert.ok(Math.abs(12 + (H - 24) * 0.74 - 31 - (H / 2 + (H - 24) * 0.24 - 31)) < 1e-9);
  }
});

/// #184 — 맥의 비활성 패널은 WKWebView :hover를 못 받으므로 엣지 펼침 판의 누를 수 있는
/// 카드 호버는 러스트 커서 폴링(hover-poll)이 .hit-hover로 그린다.
test("clickable edge cards are reported as hit regions and highlighted by hover-poll", () => {
  const report = mainSource.match(/function reportHitRegions\(\)[\s\S]*?\n\}/)?.[0] ?? "";
  // 카드가 창 전체 영역보다 먼저 — hit_region_at은 action 없는 첫 영역을 고른다
  const cards = report.indexOf("querySelectorAll<HTMLElement>(EDGE_CLICKABLE)");
  const whole = report.indexOf("regions.push({ rect: [0, 0, window.innerWidth, window.innerHeight], action: null });");
  assert.ok(cards > 0 && whole > cards, "cards must be pushed before the whole-window region");
  assert.match(mainSource, /const EDGE_CLICKABLE = "[^"]*\.edge-account\.switchable/);
  // 창 전체 영역(app)·접힌 손잡이는 호버 대상이 아니다
  const target = mainSource.match(/function polledHoverTarget\([\s\S]*?\n}\n/)?.[0] ?? "";
  assert.match(target, /return el && el !== app && el !== edgeTabEl \? el : null;/);
  assert.match(mainSource, /if \(polledHoverIdx < 0 \|\| \(locked && !edgeActive\)\) return;/);
  assert.match(cssSource, /\.edge-account\.switchable:hover,\s*\.edge-account\.switchable\.hit-hover \{/);
  // 러스트: 엣지 존이 있는 동안 hover-poll을 내고, 그동안은 표시를 거두지 않는다
  assert.match(libSource, /if !hover_poll && !edge_active && poll_hover_idx != -1 \{/);
  assert.match(libSource, /if edge_zone\.is_some\(\) \{\s*let over_idx = over\.map\(\|i\| i as i64\)\.unwrap_or\(-1\);/);
});
