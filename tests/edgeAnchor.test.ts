import assert from "node:assert/strict";
import test from "node:test";

import {
  EDGE_TAB_MARGIN,
  edgeBlobCenters,
  edgeTabOrigin,
  edgeAnchorAt,
  edgeAnchorFraction,
  edgeAnchorRange,
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

// 액체 덩어리 (2026-11-23 "애니메이션 이상해졌잖아"): 손잡이가 판 끝에 붙어도 셋이 창 안에서 번진다
const BLOB_H = 654;
const blobsInside = (b: { up: number; down: number }, h = BLOB_H) =>
  b.up - 31 >= 12 && b.up + 31 <= h - 12 && b.down - 33 >= 12 && b.down + 33 <= h - 12;

test("blobs keep the old symmetric spread when the handle is in the middle", () => {
  const b = edgeBlobCenters(BLOB_H / 2, BLOB_H);
  assert.equal(b.flipped, false);
  assert.ok(Math.abs(b.up - (BLOB_H / 2 - 630 * 0.26)) < 1e-9);
  assert.ok(Math.abs(b.down - (BLOB_H / 2 + 630 * 0.24)) < 1e-9);
});

test("the upper blob moves below when the handle sits at the top of the panel (every grip drop)", () => {
  const b = edgeBlobCenters(58.5, BLOB_H);
  assert.equal(b.flipped, true);
  assert.ok(blobsInside(b));
  assert.ok(58.5 < b.down && b.down < b.up);
});

test("the lower blob moves above when the handle sits at the bottom", () => {
  const b = edgeBlobCenters(BLOB_H - 58.5, BLOB_H);
  assert.equal(b.flipped, false);
  assert.ok(blobsInside(b));
  assert.ok(b.down < b.up && b.up < BLOB_H - 58.5);
});

test("blobs stay inside a short panel", () => {
  for (const y of [58.5, 100, 150]) assert.ok(blobsInside(edgeBlobCenters(y, 210), 210), String(y));
});

test("the handle swells from its middle when there is room, and from the window end when it sits there", () => {
  assert.equal(edgeTabOrigin(BLOB_H / 2, 109, BLOB_H), 54.5);
  for (const y of [58.5, BLOB_H - 58.5]) {
    const o = edgeTabOrigin(y, 109, BLOB_H);
    const top = y - 54.5;
    // 가장 크게 부푼(1.6배) 손잡이도 창 안에 남는다
    assert.ok(top + o * (1 - 1.6) >= -1e-9, `top ${y}`);
    assert.ok(top + o * (1 - 1.6) + 109 * 1.6 <= BLOB_H + 1e-9, `bottom ${y}`);
  }
});
