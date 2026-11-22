import assert from "node:assert/strict";
import test from "node:test";

import {
  EDGE_TAB_MARGIN,
  edgeAnchorAt,
  edgeAnchorFraction,
  edgeAnchorRange,
  edgeArrangedTop,
  edgeDropPlacement,
  edgePlacement,
  edgeTabRange,
} from "../src/edgeAnchor";

/// #188 — 엣지 손잡이의 자리. 지키는 것은 손잡이가 벽에서 서 있는 높이(anchor)이고,
/// 창은 그 손잡이를 담은 채 작업영역 안에서 필요한 만큼만 움직인다.

// 사용자 화면에서 잰 값: 작업영역 33~1153(메뉴 막대·독 제외), 판 654px, 손잡이 116px
const work = { top: 33, height: 1120 };
const height = 654;
const tabHeight = 116;
const place = (anchor: number, prevTop: number, h = height) =>
  edgePlacement({ anchor, height: h, tabHeight, work, prevTop });

test("the handle reaches both ends of the wall whatever the panel height", () => {
  const wall = edgeAnchorRange(work, tabHeight);
  assert.deepEqual(wall, { min: 33 + 58 + EDGE_TAB_MARGIN, max: 1153 - 58 - EDGE_TAB_MARGIN });
  for (const h of [200, 654, 1008]) {
    for (const prevTop of [33, 300, 1153 - h]) {
      const top = place(wall.min, prevTop, h);
      assert.equal(top.anchor, wall.min, `top end, height ${h}`);
      const bottom = place(wall.max, prevTop, h);
      assert.equal(bottom.anchor, wall.max, `bottom end, height ${h}`);
    }
  }
  // 범위 밖을 가리켜도 끝에서 멈춘다 (화면 밖으로 끌어도 손잡이는 벽 끝에 남는다)
  assert.equal(place(-5000, 300).anchor, wall.min);
  assert.equal(place(5000, 300).anchor, wall.max);
});

test("the window always stays inside the work area and always contains the handle", () => {
  const wall = edgeAnchorRange(work, tabHeight);
  for (const h of [160, 420, 654, 1008, 1120]) {
    const tab = edgeTabRange(h, tabHeight);
    for (let anchor = wall.min; anchor <= wall.max; anchor += 37) {
      for (const prevTop of [-400, 33, 250, 499, 2000]) {
        const placed = place(anchor, prevTop, h);
        assert.ok(placed.top >= work.top - 1e-9, `top inside (h=${h}, anchor=${anchor}, prev=${prevTop})`);
        assert.ok(placed.top + h <= work.top + work.height + 1e-9, `bottom inside (h=${h}, anchor=${anchor})`);
        assert.ok(placed.tabY >= tab.min - 1e-9 && placed.tabY <= tab.max + 1e-9, "handle inside the window");
        assert.ok(Math.abs(placed.top + placed.tabY - anchor) < 1e-9, "the handle stands where it was put");
      }
    }
  }
});

test("dragging moves the handle inside the window first and takes the window along only at its end", () => {
  // 창 269~923, 손잡이는 창 아래 끝(사용자가 갇혀 있던 상태: 화면 858)
  let state = place(269 + 589, 269);
  assert.deepEqual([state.top, state.tabY], [269, 589]);
  // 위로 400px: 손잡이가 창 안에서 올라간다 — 창은 그대로
  state = place(858 - 400, state.top);
  assert.deepEqual([state.top, state.tabY, state.anchor], [269, 189, 458]);
  // 300px 더: 창 위 끝(62)에 닿은 뒤로는 창이 따라 올라간다
  state = place(458 - 300, state.top);
  assert.deepEqual([state.top, state.tabY, state.anchor], [96, 62, 158]);
  // 맨 위까지: 창은 작업영역 위에서 멈추고 손잡이는 벽 위 끝에 선다
  state = place(0, state.top);
  assert.deepEqual([state.top, state.tabY, state.anchor], [33, 62, 95]);
  // 다시 아래 끝까지 한 번에
  state = place(5000, state.top);
  assert.deepEqual([state.top, state.tabY, state.anchor], [499, 592, 1091]);
});

test("a taller or shorter panel leaves the handle where it stands on screen", () => {
  // 작업영역 바닥에 붙은 창(499~1153)의 손잡이가 화면 1000에 있다
  const before = place(1000, 499);
  assert.deepEqual([before.top, before.tabY], [499, 501]);
  // API 칸이 생겨 판이 60px 커진다 — 창이 위로 60px 자라고 손잡이는 화면에서 그대로
  const taller = place(1000, before.top, height + 60);
  assert.deepEqual([taller.top, taller.tabY, taller.anchor], [439, 561, 1000]);
  // 다시 줄어든다 — 손잡이를 담을 수 있으니 창은 그 자리, 손잡이도 그대로
  const shorter = place(1000, taller.top, height);
  assert.deepEqual([shorter.top, shorter.tabY, shorter.anchor], [439, 561, 1000]);
  // 훨씬 짧아져 손잡이가 창 밖이 되면 창이 손잡이 쪽으로 내려온다
  const tiny = place(1000, shorter.top, 300);
  assert.equal(tiny.anchor, 1000);
  assert.equal(tiny.top + tiny.tabY, 1000);
  assert.equal(tiny.tabY, edgeTabRange(300, tabHeight).max);
});

test("the window returns to the user's arrangement once nothing presses it", () => {
  // 화면 코드는 "판 안에서의 손잡이 자리"(여기서는 501)가 되는 창 위치에서 출발한다:
  // prevTop = anchor - 501. 판이 커져 바닥에 눌리면 창이 올라가고, 줄어들면 제자리로 돌아온다.
  const anchor = 1000;
  const preferred = anchor - 501;
  assert.deepEqual(place(anchor, preferred).top, 499);
  assert.deepEqual(place(anchor, preferred, height + 60).top, 439);
  assert.deepEqual(place(anchor, preferred, height).top, 499);
  // 다시 띄운 직후 창이 화면 아래 구석에 있어도, 높이가 작은 골격에서 시작해도 결과는 같다
  const small = place(anchor, preferred, 200);
  assert.equal(small.top + small.tabY, anchor);
  assert.deepEqual([place(anchor, preferred, height).top, place(anchor, preferred, height).tabY], [499, 501]);
});

test("a panel dropped past the work area comes back alone; the handle stays where it was dropped", () => {
  // ☰로 판을 작업영역 아래로 150px 넘겨 놓았다 (창 649~1303, 손잡이는 창 중앙 = 화면 976)
  const dropped = place(649 + 327, 649);
  assert.deepEqual([dropped.top, dropped.tabY, dropped.anchor], [499, 477, 976]);
});

test("the stored height is a share of the wall, so it survives a different screen or dock size", () => {
  const wall = edgeAnchorRange(work, tabHeight);
  assert.equal(edgeAnchorFraction(wall.min, work, tabHeight), 0);
  assert.equal(edgeAnchorFraction(wall.max, work, tabHeight), 1);
  for (const anchor of [95, 300, 593, 858, 1091]) {
    const back = edgeAnchorAt(edgeAnchorFraction(anchor, work, tabHeight), work, tabHeight);
    assert.ok(Math.abs(back - anchor) < 1e-9, `round trip at ${anchor}`);
  }
  // 독이 커져 작업영역이 줄어도 끝에 둔 손잡이는 끝에 남는다
  const smaller = { top: 33, height: 1000 };
  assert.equal(edgeAnchorAt(0, smaller, tabHeight), edgeAnchorRange(smaller, tabHeight).min);
  assert.equal(edgeAnchorAt(1, smaller, tabHeight), edgeAnchorRange(smaller, tabHeight).max);
  // 범위 밖 저장값은 끝으로 읽는다
  assert.equal(edgeAnchorAt(7, work, tabHeight), wall.max);
  assert.equal(edgeAnchorAt(-3, work, tabHeight), wall.min);
  // 손잡이가 벽 전체를 차지해 움직일 곳이 없으면 가운데로 본다
  assert.equal(edgeAnchorFraction(50, { top: 0, height: 100 }, 116), 0.5);
});

/// ☰로 판을 옮겨 놓았을 때 (#188). 사용자 지시: "마우스는 저 위에 있는데 작아진 창은 아래 있어봐야",
/// 이어서 "위는 되어도 아래가" — 위와 아래가 똑같이 되어야 한다.
// 사용자 화면: 작업영역 38~1153, 판 654px, 손잡이 109px (끝에서 58.5px)
const screen = { top: 38, height: 1115 };
const drop = (dropTop: number, cursor: number | null, grip: "top" | "bottom", h = 654) =>
  edgeDropPlacement({ dropTop, height: h, tabHeight: 109, work: screen, cursor, grip });
const END = 58.5;

test("dropped near the top, the small handle is at the mouse and the panel hangs below it", () => {
  // 위쪽 ☰(판 위에서 25px)를 쥐고 화면 300 높이에 놓았다 — 판은 놓인 자리 그대로, 손잡이는 그 위쪽 끝
  const placed = drop(275, 300, "top");
  assert.deepEqual([placed.top, placed.tabY, placed.end], [275, END, "top"]);
  // 손잡이(109px)가 마우스를 품는다
  assert.ok(Math.abs(placed.anchor - 300) <= 109 / 2);
  // 화면 맨 위: OS가 창을 메뉴 막대 밑에서 막아도 손잡이는 벽 위 끝
  const top = drop(38, 5, "top");
  assert.deepEqual([top.top, top.tabY, top.anchor, top.end], [38, END, 96.5, "top"]);
});

test("dropped near the bottom, the handle is at the mouse and the panel stands above it", () => {
  // 위쪽 ☰를 쥔 채 화면 900 높이까지 끌어 놓았다 (판은 875~1529로 화면 밖) —
  // 손잡이는 마우스 자리에, 판은 손잡이 위로 열린다(아래쪽 ☰가 손잡이 옆에 온다)
  const placed = drop(875, 900, "top");
  assert.equal(placed.end, "bottom");
  assert.equal(placed.tabY, 654 - END);
  assert.ok(Math.abs(placed.anchor - 900) <= 109 / 2, "the handle contains the mouse");
  assert.ok(placed.top >= 38 && placed.top + 654 <= 1153, "the panel is back on screen");
  // 화면 맨 아래: 손잡이는 벽 아래 끝, 판은 바닥에 선다
  const bottom = drop(1075, 1100, "top");
  assert.deepEqual([bottom.top, bottom.tabY, bottom.anchor, bottom.end], [499, 654 - END, 1094.5, "bottom"]);
});

test("moving a low handle with the bottom grip moves it by exactly that much, down or up", () => {
  // 손잡이가 화면 900에 있고 판은 그 위로 서 있다. 아래쪽 ☰(판 아래에서 25px)를 쥔다.
  const start = drop(875, 900, "top");
  const gripY = start.top + 654 - 25;
  // 80px 내린다 — 판도 손잡이도 80px 내려간다 (위로 튀지 않는다)
  const down = drop(start.top + 80, gripY + 80, "bottom");
  assert.deepEqual([down.top, down.end], [start.top + 80, "bottom"]);
  assert.equal(down.anchor, start.anchor + 80);
  // 80px 올린다
  const up = drop(start.top - 80, gripY - 80, "bottom");
  assert.deepEqual([up.top, up.end], [start.top - 80, "bottom"]);
  assert.equal(up.anchor, start.anchor - 80);
  // 바닥 너머까지 내리면 벽 아래 끝에서 멈춘다
  const floor = drop(start.top + 400, gripY + 400, "bottom");
  assert.deepEqual([floor.top, floor.anchor, floor.end], [499, 1094.5, "bottom"]);
});

test("a low handle can be carried up into the top half in one drag", () => {
  // 판이 손잡이 위로 서 있다(38~692, 손잡이 633.5). 아래쪽 ☰를 쥐고 위로 민다 — OS가 창을 메뉴 막대
  // 밑에서 막아 창은 더 못 가지만 마우스는 300까지 올라갔다. 손잡이는 마우스 자리, 판은 그 아래로.
  const placed = drop(38, 300, "bottom");
  assert.deepEqual([placed.anchor, placed.end, placed.tabY], [300, "top", END]);
  assert.equal(placed.top, 300 - END);
  // 맨 위까지
  const top = drop(38, 2, "bottom");
  assert.deepEqual([top.top, top.anchor, top.end], [38, 96.5, "top"]);
});

test("without the pointer record the handle goes to the end that was held", () => {
  assert.deepEqual([drop(275, null, "top").anchor, drop(275, null, "top").end], [275 + END, "top"]);
  // 아래쪽 ☰를 쥐었으면 판 아래쪽 끝 — 그 자리에서 판이 위로도 아래로도 들어가면 놓인 자리 그대로
  const held = drop(200, null, "bottom");
  assert.deepEqual([held.top, held.anchor, held.end], [200, 200 + 654 - END, "bottom"]);
});

test("a short panel that fits either way stays where it was dropped", () => {
  // 300px 판: 손잡이 위로도 아래로도 들어간다 — 놓인 자리에서 가까운 쪽(쥔 끝)을 고른다
  const viaTop = drop(500, 525, "top", 300);
  assert.deepEqual([viaTop.top, viaTop.end], [500, "top"]);
  const viaBottom = drop(500, 500 + 300 - 25, "bottom", 300);
  assert.deepEqual([viaBottom.top, viaBottom.end], [500, "bottom"]);
});

test("where neither end fits, the panel is only brought on screen and the handle stays at the mouse", () => {
  // 판이 화면의 절반보다 길면 가운데 구간(여기서는 557.5~633.5)에서는 어느 끝에도 붙지 못한다
  const placed = drop(545, 570, "top");
  assert.equal(placed.anchor, 545 + END);
  assert.equal(placed.top, 499);
  assert.equal(placed.top + placed.tabY, placed.anchor);
  assert.equal(placed.end, "top");
});

test("the stored arrangement is kept when the panel changes height", () => {
  // 손잡이 위로 서 있던 판(아래쪽 끝에서 58.5)은 커져도 아래쪽 끝이 손잡이에 붙어 있다
  assert.equal(edgeArrangedTop(900, 654, "bottom", END), 900 + END - 654);
  assert.equal(edgeArrangedTop(900, 714, "bottom", END), 900 + END - 714);
  // 손잡이 아래로 열리던 판은 위쪽 끝이 그대로다
  assert.equal(edgeArrangedTop(300, 654, "top", END), 300 - END);
  assert.equal(edgeArrangedTop(300, 714, "top", END), 300 - END);
  // 화면 밖으로 나가는 만큼은 edgePlacement가 들인다 — 손잡이는 제자리
  const placed = edgePlacement({ anchor: 540, height: 714, tabHeight: 109, work: screen, prevTop: edgeArrangedTop(540, 714, "top", END) });
  assert.deepEqual([placed.top, placed.anchor], [439, 540]);
});
