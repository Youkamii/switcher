import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

test("mac brightness sliders never reach 0, which turns the backlight off (#177)", () => {
  assert.match(
    mainSource,
    /const IS_MAC = navigator\.platform\.startsWith\("Mac"\);/,
    "the slider floor must use the same platform check that sets body.mac",
  );
  assert.match(
    mainSource,
    /document\.body\.classList\.toggle\("mac", IS_MAC\);/,
    "body.mac must be derived from the same IS_MAC constant",
  );
  assert.match(
    mainSource,
    /const BRIGHTNESS_MIN = IS_MAC \? 1 : 0;/,
    "macOS floor is 1 (DisplayServices 0 = backlight off); Windows keeps 0",
  );
  // 밝기 슬라이더를 만드는 두 함수 본문 안에서만 검사한다 — 다른 range 입력이 0 하한을
  // 쓰는 것까지 막지 않는다 (review)
  const brightnessSource = [
    /async function renderDisplays\([\s\S]*?\n}\n/,
    /async function renderDisplaysEdge\([\s\S]*?\n}\n/,
  ]
    .map((pattern) => mainSource.match(pattern)?.[0] ?? "")
    .join("\n");
  assert.ok(brightnessSource.length > 0, "both brightness render functions must exist");
  assert.doesNotMatch(
    brightnessSource,
    /slider\.min = "0"/,
    "no brightness slider may hard-code a 0 floor",
  );
  const floors = mainSource.match(/slider\.min = String\(BRIGHTNESS_MIN\);/g) ?? [];
  assert.equal(floors.length, 2, "both the Type1/compact and the Type4 sliders use the floor");
  const clamps = mainSource.match(/const start = Math\.max\(BRIGHTNESS_MIN, monitor\.brightness/g) ?? [];
  assert.equal(clamps.length, 2, "both sliders clamp the initial value to the floor");
  const starts = mainSource.match(/slider\.value = String\(start\);/g) ?? [];
  assert.equal(starts.length, 2, "both sliders start from the clamped value");
});
