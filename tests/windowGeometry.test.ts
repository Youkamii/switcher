import assert from "node:assert/strict";
import test from "node:test";
import {
  clampWindowToWorkArea,
  edgeSnapPosition,
  logicalWorkAreaHeight,
  monitorGeometryKey,
  pickEdgeSide,
} from "../src/windowGeometry.ts";

test("converts a physical work area using the destination monitor scale", () => {
  const area = { position: { x: 1920, y: 0 }, size: { width: 2560, height: 1400 } };
  assert.equal(logicalWorkAreaHeight(area, 1.25), 1120);
});

test("falls back to physical height when a monitor reports an invalid scale", () => {
  const area = { position: { x: 0, y: 0 }, size: { width: 1920, height: 1032 } };
  for (const scale of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(logicalWorkAreaHeight(area, scale), 1032);
  }
});

test("monitor key changes for aspect ratio and scale changes", () => {
  const landscape = { position: { x: 0, y: 0 }, size: { width: 1920, height: 1032 } };
  const portrait = { position: { x: 1920, y: 136 }, size: { width: 1080, height: 1872 } };
  assert.notEqual(monitorGeometryKey(landscape, 1), monitorGeometryKey(portrait, 1));
  assert.notEqual(
    monitorGeometryKey(landscape, 1),
    monitorGeometryKey({ ...landscape, size: { ...landscape.size, width: 1080 } }, 1),
  );
  assert.notEqual(
    monitorGeometryKey(landscape, 1),
    monitorGeometryKey({ ...landscape, size: { ...landscape.size, height: 1872 } }, 1),
  );
  assert.notEqual(
    monitorGeometryKey(landscape, 1),
    monitorGeometryKey({ ...landscape, position: { ...landscape.position, x: -1920 } }, 1),
  );
  assert.notEqual(
    monitorGeometryKey(landscape, 1),
    monitorGeometryKey({ ...landscape, position: { ...landscape.position, y: 136 } }, 1),
  );
  assert.notEqual(monitorGeometryKey(landscape, 1), monitorGeometryKey(landscape, 1.25));
});

test("keeps the whole widget inside a shorter destination work area", () => {
  const area = { position: { x: 0, y: 0 }, size: { width: 1920, height: 1032 } };
  assert.deepEqual(
    clampWindowToWorkArea({ x: 1528, y: 467, width: 376, height: 678 }, area),
    { x: 1528, y: 354 },
  );
});

test("clamps correctly in a negative-coordinate monitor", () => {
  const area = { position: { x: -1920, y: 677 }, size: { width: 1920, height: 1032 } };
  assert.deepEqual(
    clampWindowToWorkArea({ x: -2000, y: 1600, width: 376, height: 400 }, area),
    { x: -1920, y: 1309 },
  );
});

test("clamps every edge and leaves an in-bounds widget unchanged", () => {
  const area = { position: { x: 100, y: 50 }, size: { width: 1000, height: 700 } };
  const cases = [
    [{ x: 950, y: 700, width: 200, height: 100 }, { x: 900, y: 650 }],
    [{ x: 50, y: 25, width: 200, height: 100 }, { x: 100, y: 50 }],
    [{ x: 400, y: 300, width: 200, height: 100 }, { x: 400, y: 300 }],
    [{ x: 400, y: 300, width: 1200, height: 900 }, { x: 100, y: 50 }],
  ] as const;

  for (const [windowRect, expected] of cases) {
    const clamped = clampWindowToWorkArea(windowRect, area);
    assert.deepEqual(clamped, expected);
    assert.deepEqual(
      clampWindowToWorkArea({ ...windowRect, ...clamped }, area),
      clamped,
    );
  }
});

test("Type4 picks the wall nearest to the window center", () => {
  const area = { position: { x: 1920, y: 0 }, size: { width: 2560, height: 1400 } };
  assert.equal(pickEdgeSide({ x: 2000, y: 100, width: 240, height: 500 }, area), "left");
  assert.equal(pickEdgeSide({ x: 3100, y: 100, width: 240, height: 500 }, area), "right");
  // 중심선 위는 오른쪽(기본 벽)
  assert.equal(pickEdgeSide({ x: 3080, y: 100, width: 240, height: 500 }, area), "right");
});

test("Type4 snap sticks the window flush to the wall and keeps it inside vertically", () => {
  const area = { position: { x: 1920, y: 40 }, size: { width: 2560, height: 1400 } };
  assert.deepEqual(
    edgeSnapPosition({ x: 2500, y: 100, width: 240, height: 500 }, area, "right"),
    { x: 1920 + 2560 - 240, y: 100 },
  );
  assert.deepEqual(
    edgeSnapPosition({ x: 2500, y: -50, width: 240, height: 500 }, area, "left"),
    { x: 1920, y: 40 },
  );
  assert.deepEqual(
    edgeSnapPosition({ x: 2500, y: 5000, width: 240, height: 500 }, area, "left"),
    { x: 1920, y: 40 + 1400 - 500 },
  );
});

test("Type4 snap pushes the invisible shadow margin past the wall", () => {
  const area = { position: { x: 0, y: 0 }, size: { width: 1920, height: 1032 } };
  const win = { x: 1000, y: 451, width: 256, height: 581 };
  assert.deepEqual(edgeSnapPosition(win, area, "right", { left: 8, right: 8 }), {
    x: 1920 - 256 + 8,
    y: 451,
  });
  assert.deepEqual(edgeSnapPosition(win, area, "left", { left: 8, right: 8 }), {
    x: -8,
    y: 451,
  });
});
