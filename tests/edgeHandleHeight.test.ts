import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const mainSource = read("../src/main.ts");
const cssSource = read("../src/styles.css");
const libSource = read("../src-tauri/src/lib.rs");

/// #185·#188 — 엣지(벽 붙임) 손잡이의 자리. 지키는 것은 손잡이가 벽에서 서 있는 높이이고
/// (계산은 edgeAnchor.ts, tests/edgeAnchor.test.ts), 여기서는 그 계산이 화면 코드의 어느
/// 길목에 물려 있는지를 본다.
const fn = (name: string) => mainSource.match(new RegExp(`(?:async )?function ${name}\\([\\s\\S]*?\\n}\\n`))?.[0] ?? "";

test("a panel moved with the grip leaves the small handle where the mouse let go", () => {
  // ☰를 누른 뒤 정착한 이동만 사용자의 판 옮기기다 — 크기 맞춤·손잡이 끌기·OS가 옮긴 창은
  // 저장된 자리를 기준으로 다시 놓는다
  const settle = fn("settleMonitorSync");
  assert.match(settle, /if \(edgeGripPressed\) \{\s*edgeGripPressed = false;\s*edgeGripDrop = edgeActive;\s*\}\s*void syncCurrentMonitor\(forceSync\);/);
  // ☰를 누르면: 러스트에 그 누름의 기록을 걸어 두고, 누를 때의 창 위치를 적고, 창이 움직이지
  // 않아도(이미 화면 끝) 놓은 뒤 한 번은 정착 판정이 돈다
  const press = mainSource.match(/if \(target\?\.closest\("\.edge-handle"\)\) \{[\s\S]*?return;\s*\}/)?.[0] ?? "";
  assert.match(press, /edgeGripPressed = true;/);
  assert.match(press, /void invoke\("arm_grip_drop"\)/);
  assert.match(press, /if \(!edgeGripPressed\) return;\s*edgeGripStart = \{ x: at\.x, y: at\.y \};/);
  // 사용자 지시(2026-11-23): 벽에 붙인 채로 마우스 높이를 따라 움직인다. 네이티브 창 드래그는 판을
  // 벽 밖으로 밀어 넣으므로 쓰지 않는다 — 러스트가 두 벽의 가로 자리와 세로 범위 안에서 창을 옮긴다
  assert.doesNotMatch(fn("edgeBar"), /data-tauri-drag-region/);
  assert.match(press, /invoke\("set_grip_rail", \{\s*rail: \{\s*xLeft: edgeSnapPosition\(rect, area, "left", insets\)\.x,\s*xRight: edgeSnapPosition\(rect, area, "right", insets\)\.x,\s*yMin,\s*yMax: [^\n]*\n\s*midX: /);
  assert.match(libSource, /fn grip_rail_position\(rail: &GripRail, cursor: \(f64, f64\), grab: f64\) -> \(i32, i32, bool\)/);
  assert.match(libSource, /let rail = if GRIP_ARMED\.load\(Ordering::Relaxed\) && primary_button_down\(\) \{/);
  assert.match(libSource, /\n\s+set_grip_rail,\n/, "the command must be registered");
  assert.match(mainSource, /listen<string>\("edge-grip-side"/);
  assert.match(press, /scheduleMonitorSync\(\);/);
  const snap = fn("placeOnEdge");
  // 정착 판정이 세운 표시를 스냅이 거둔다 — 그 사이에 다른 길로 온 스냅이 옛 자리로 되돌리지 못한다
  assert.match(snap, /const gripSettled = edgeGripDrop;\s*edgeGripDrop = false;/);
  // 끌었는가: 커서가 움직였거나(창이 화면 끝에 막혀 못 움직였어도 끈 것이다) 창이 움직였거나.
  // 누르기만 한 ☰는 손잡이를 옮기지 않는다
  assert.match(snap, /await invoke<GripDrop \| null>\("take_grip_drop"\)/);
  assert.match(snap, /Math\.hypot\(press\.up\[0\] - press\.down\[0\], press\.up\[1\] - press\.down\[1\]\) >= EDGE_GRIP_MOVE_MIN \* scale;/);
  assert.match(snap, /dragged = cursorMoved \|\| windowMoved;/);
  // 사용자 지시(2026-11-23): 잡고 끌어 놓으면 접힌 손잡이의 맨 위가 끌어 놓은 자리에 온다.
  // 놓은 높이는 러스트 기록이 이 ☰ 누름의 것일 때만 쓰고, 아니면 놓인 판의 맨 위다.
  assert.match(snap, /if \(Math\.abs\(press\.down\[1\] \/ scale - pressedAt\) <= EDGE_GRIP_TRUST\) released = press\.up\[1\] \/ scale;/);
  assert.match(
    snap,
    /const tabTop = released \?\? frame\.top \+ EDGE_TAB_MARGIN;\s*storeEdgeAnchor\(tabTop \+ edgeTabHeight \/ 2, frame\.work\);\s*localStorage\.setItem\("switcher\.edgeside", edgeSide\);/,
  );
  assert.doesNotMatch(snap, /pointer_position/);
  // 러스트: 걸어 둔 누름이 끝나면 누른 자리·놓은 자리를 남긴다 (버튼을 쥔 채 마지막으로 본 커서)
  assert.match(libSource, /fn arm_grip_drop\(\)/);
  assert.match(libSource, /fn take_grip_drop\(\) -> Option<PointerPress>/);
  assert.match(libSource, /\n\s+arm_grip_drop,\n\s+take_grip_drop,\n/, "the commands must be registered");
  assert.match(libSource, /if let Some\(press\) = presses\.update\(primary_button_down\(\), global_cursor_pos\(\)\) \{\s*if GRIP_ARMED\.swap\(false, Ordering::Relaxed\) \{/);
});

test("every snap puts the window around the stored handle height, so a resized panel does not carry the handle away", () => {
  const snap = fn("placeOnEdge");
  // 자리가 정해져 있으면 창 위치는 그 자리에서 나온다 — 단순한 작업영역 clamp가 아니다
  assert.match(
    snap,
    /if \(edgeAnchor !== null\) \{\s*const anchor = edgeAnchorAt\(edgeAnchor, frame\.work, edgeTabHeight\);\s*const placed = edgePlacement\(\{\s*anchor,\s*height: frame\.height,\s*tabHeight: edgeTabHeight,\s*work: frame\.work,/,
  );
  // 창은 사용자가 정한 "판 안에서의 손잡이 자리"가 되는 곳에서 출발한다 — 다시 띄운 직후에도,
  // 판이 커져 화면 끝에 눌렸다 줄어든 뒤에도 같은 모양으로 돌아온다. 방금 놓은 판만 놓인 자리에서.
  assert.match(snap, /prevTop: dragged \? anchor - edgeTabRange\(frame\.height, edgeTabHeight\)\.min : edgeTabY === null \? frame\.top : anchor - edgeTabY,/);
  // 눌려서 달라진 자리는 그리기만 하고 저장하지 않는다 — 저장은 사용자가 옮겼을 때만
  assert.match(snap, /target\.y = innerY - frame\.insetTop;\s*showEdgeTab\(placed\.anchor - innerY \/ scale\);\s*if \(dragged\) \{\s*(?:\/\/[^\n]*\n\s*)*edgeTabY = edgeTabRange\(frame\.height, edgeTabHeight\)\.min;/);
  assert.doesNotMatch(fn("showEdgeTab"), /localStorage/);
  // 크기를 맞춘 뒤의 스냅이 이 길로 온다
  assert.match(mainSource, /if \(edgeActive\) await snapToEdge\(\);/);
  // 손잡이를 끄는 동안과 그 창 이동이 끝나기 전에는 스냅이 끼어들지 않는다
  assert.match(snap, /edgeTabDrag\?\.active === true \|\| edgeWindowMove\.busy/);
});

test("the handle height, the wall and the in-window offset survive a restart", () => {
  // 앱은 늘 오른쪽 아래에서 뜬다 — 저장하지 않으면 다시 띄울 때마다 손잡이가 아래로 돌아간다
  assert.match(libSource, /position_bottom_right\(&window\);/);
  assert.match(mainSource, /localStorage\.getItem\("switcher\.edgeanchor"\)/);
  assert.match(fn("storeEdgeAnchor"), /edgeAnchor = edgeAnchorFraction\(anchor, work, edgeTabHeight\);\s*localStorage\.setItem\("switcher\.edgeanchor", edgeAnchor\.toFixed\(5\)\);/);
  assert.match(fn("storeEdgeTabY"), /edgeTabY = edgeTabShown;\s*localStorage\.setItem\("switcher\.edgetab", String\(edgeTabY\)\);/);
  // 지난번 벽: 엣지에 들어오면 첫 스냅이 창이 놓인 쪽이 아니라 저장된 벽으로 간다
  assert.match(fn("applyEdgeMode"), /const side = storedEdgeSide\(\);\s*if \(side\) \{\s*edgeSide = side;\s*edgeRestoreSide = true;/);
  const place = fn("placeOnEdge");
  assert.match(place, /const side = edgeRestoreSide \? edgeSide : pickEdgeSide\(rect, currentWorkArea\);/);
  // 창 이동은 호출이 끝난 뒤에야 반영된다 — 그 벽에 와 있는 것을 본 뒤에야 복원을 마친다. 옮기라고 한
  // 직후에 지우면 다음 스냅이 옮기기 전 위치를 읽고 반대 벽으로 되돌린다 (실제 앱에서 본 결함)
  assert.match(place, /if \(edgeRestoreSide && target\.x === pos\.x\) edgeRestoreSide = false;/);
  assert.doesNotMatch(place, /setPosition[\s\S]*edgeRestoreSide = false/);
  // 사용자가 ☰로 벽을 골랐으면 지난번 벽으로 되돌리지 않는다
  assert.match(place, /if \(dragged\) edgeRestoreSide = false;/);
  assert.match(place, /localStorage\.setItem\("switcher\.edgeside", edgeSide\);/);
  // 스냅은 한 번에 하나만 돈다 — 겹친 요청은 끝난 뒤 한 번 더
  const serial = fn("snapToEdge");
  assert.match(serial, /if \(edgeSnapRun\) \{\s*edgeSnapAgain = true;\s*return edgeSnapRun;/);
  assert.match(serial, /do \{\s*edgeSnapAgain = false;\s*await placeOnEdge\(\);\s*\} while \(edgeSnapAgain\);/);
  // 엣지를 떠나면 남은 표시를 지운다 — 다른 모드에서의 창 위치를 드롭으로 읽지 않는다
  assert.match(fn("applyEdgeMode"), /edgeGripPressed = false;\s*edgeGripDrop = false;\s*edgeGripStart = null;\s*edgeRestoreSide = false;/);
  assert.match(fn("finishEdgeTabDrag"), /storeEdgeAnchor\(drag\.anchor, drag\.geo\.work\);\s*storeEdgeTabY\(\);\s*localStorage\.setItem\("switcher\.edgeside", edgeSide\);/);
});

test("the handle stays inside the window for any gauge height", () => {
  // 손잡이 키는 게이지 줄 수에 따라 변한다 — 범위는 현재 키로 계산한다
  assert.match(fn("edgeTabBounds"), /return edgeTabRange\(window\.innerHeight, edgeTabHeight\);/);
  // 키가 바뀌면(계정·SYSTEM 줄 변화) 저장된 높이를 다시 가두고, 벽 끝에 둔 손잡이는 끝에 다시 붙인다
  assert.match(
    mainSource,
    /style\.setProperty\("--edge-tab-height", `\$\{geometry\.tabHeight\}px`\);\s*(?:\/\/[^\n]*\n\s*)*edgeTabHeight = geometry\.tabHeight;\s*applyEdgeTabY\(\);\s*(?:\/\/[^\n]*\n\s*)*if \(edgeActive && edgeAnchor !== null\) void snapToEdge\(\)\.catch/,
  );
  // 창 높이가 바뀌면 다시 누른다 — 비활성 웹뷰 타이머가 아니라 resize 이벤트
  assert.match(mainSource, /window\.addEventListener\("resize", \(\) => \{\s*if \(edgeActive\) applyEdgeTabY\(\);/);
});

/// #188 — 접힌(작은) 손잡이를 누른 채 위아래로 끈다 (사용자 지시).
test("pressing the small handle and dragging moves it along the wall", () => {
  // 커서가 닿으면 판이 펼쳐지므로 "손잡이가 있던 자리에서 시작한 세로 끌기"로 알아본다
  const zone = fn("inEdgeTabZone");
  assert.match(zone, /Math\.abs\(y - edgeTabCenter\(\)\) > half/);
  assert.match(zone, /return edgeSide === "left" \? x <= reach : x >= window\.innerWidth - reach;/);
  // 손잡이를 겨냥한 누름(내용이 보이기 전)은 4px, 보이는 내용 위에서 시작한 누름은 12px를 넘겨야 끌기다
  assert.match(mainSource, /const EDGE_TAB_DRAG_SLOP = 4;\s*const EDGE_TAB_DRAG_SLOP_OPEN = 12;/);
  assert.match(mainSource, /slop: edgePressEarly \? EDGE_TAB_DRAG_SLOP : EDGE_TAB_DRAG_SLOP_OPEN,/);
  assert.match(mainSource, /edgePressEarly = !edgeOut \|\| Date\.now\(\) - edgeShownAt < EDGE_CONTENT_DELAY_MS;/);
  // 끌기가 되면 판을 연출 없이 접어 손잡이를 보이고, 러스트에 끄는 중임을 알린다
  const begin = fn("beginEdgeTabDrag");
  assert.match(begin, /edgeSuppressClick = true;/);
  assert.match(begin, /setEdgeDragHold\(true\);/);
  assert.match(begin, /edgeOut = false;\s*document\.body\.classList\.add\("edge-noanim", "edge-dragging"\);\s*document\.body\.classList\.remove\("edge-out"\);/);
  // 손잡이는 CSS로 바로, 창은 따라와야 할 때만 — 화면 좌표의 이동량을 쓴다(창이 움직여도 흔들리지 않는다)
  const pump = fn("pumpEdgeTabDrag");
  assert.match(pump, /anchor: geo\.startAnchor \+ \(drag\.lastY - drag\.startY\),/);
  assert.match(pump, /prevTop: drag\.top,/);
  assert.match(pump, /applyEdgeTabY\(\);\s*moveEdgeWindow\(innerY - geo\.insetTop\);/);
  assert.match(mainSource, /drag\.lastY = event\.screenY;/);
  // 창 이동은 한 번에 하나, 밀린 것은 마지막 목표만
  assert.match(fn("moveEdgeWindow"), /while \(edgeWindowMove\.want !== null && edgeWindowMove\.want !== edgeWindowMove\.sent\)/);
  assert.match(cssSource, /body\.edge\.edge-dragging #edge-tab,\s*body\.edge\.edge-dragging #edge-gauge \{\s*animation: none;\s*transition: none;/);
});

test("a press meant for the handle never activates the card under it", () => {
  // 끌기가 된 누름·내용이 보이기 전에 시작한 누름의 click은 캡처 단계에서 삼킨다 —
  // 그 자리의 카드가 계정(또는 유료 API 키)으로 넘기면 안 된다
  assert.match(
    mainSource,
    /const swallow = edgeActive && fromPress && \(edgeSuppressClick \|\| edgePressEarly\);[\s\S]*?if \(!swallow\) return;\s*event\.stopPropagation\(\);\s*event\.preventDefault\(\);\s*\},\s*true,\s*\);/,
  );
  // 내용이 보이는 시점은 CSS의 등장 지연과 같은 값이다
  assert.match(mainSource, /const EDGE_CONTENT_DELAY_MS = 700;/);
  assert.match(cssSource, /body\.edge\.edge-out \.shell \{\s*animation: edge-content-in 0\.22s 0\.7s ease-out both;/);
  assert.match(fn("setEdgeOut"), /if \(out\) edgeShownAt = edgeAnimStartedAt;/);
  // 링크·그림의 HTML 드래그가 되면 포인터 이벤트가 끊긴다
  assert.match(mainSource, /"dragstart",\s*\(event\) => \{\s*if \(edgeActive && edgeTabDrag\) event\.preventDefault\(\);/);
});

test("the window keeps the mouse while the handle is dragged, and the drag always ends", () => {
  // 끄는 동안 폴링은 투과로 바꾸지 않는다 — 커서는 손잡이 영역을 벗어난다
  assert.match(libSource, /static EDGE_TAB_DRAG: AtomicBool = AtomicBool::new\(false\);/);
  assert.match(libSource, /fn set_edge_drag\(active: bool\)/);
  assert.match(libSource, /\n\s+set_edge_drag,\n/, "the command must be registered");
  const loop = libSource.match(/if EDGE_TAB_DRAG\.load\(Ordering::Relaxed\) \{[\s\S]*?continue;\s*\}\s*edge_drag_up_ticks = 0;/)?.[0] ?? "";
  assert.match(loop, /if ignoring \{\s*let _ = window\.set_ignore_cursor_events\(false\);\s*ignoring = false;/);
  assert.match(loop, /edge_hover\.hold_inside\(std::time::Instant::now\(\)\);/);
  // 웹뷰가 놓는 이벤트를 못 받아도 러스트가 끝낸다 (창이 마우스를 잡은 채 남지 않는다)
  assert.match(loop, /if edge_drag_released\(down, &mut edge_drag_up_ticks\) \{\s*EDGE_TAB_DRAG\.store\(false, Ordering::Relaxed\);[\s\S]*?handle\.emit\("edge-drag-end", \(\)\);/);
  assert.match(mainSource, /void listen\("edge-drag-end", \(\) => finishEdgeTabDrag\(\)\);/);
  // 버튼이 떨어진 채 온 이동도 끝으로 본다
  assert.match(mainSource, /if \(\(event\.buttons & 1\) === 0\) \{\s*finishEdgeTabDrag\(\);/);
  // 엣지를 떠나면 끌기도 끝이다
  assert.match(fn("applyEdgeMode"), /endEdgeHold\(\);\s*finishEdgeTabDrag\(\);/);
  assert.match(libSource, /if zone\.is_none\(\) \{\s*(?:\/\/[^\n]*\n\s*)*EDGE_TAB_DRAG\.store\(false, Ordering::Relaxed\);/);
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
