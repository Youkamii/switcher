import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const htmlSource = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

/// 주석을 걷어낸 CSS — 셀렉터 캡처(`[^{}]+`)가 바로 앞 주석을 통째로 삼키기
/// 때문이다. styles.css의 주석에는 #pin·#drag-handle 같은 ID가 그대로 적혀
/// 있어, 걷어내지 않으면 아래 헬퍼가 주석을 셀렉터로 착각한다.
const css = stylesSource.replace(/\/\*[\s\S]*?\*\//g, "");

/// 선언 블록을 셀렉터로 훑어 특정 속성의 값들을 모은다 — "어느 규칙에서든
/// 이 속성이 이 값 말고 다른 값으로 덮이지 않는다"를 검사하기 위한 것
function declaredValues(selectorPart: string, property: string): string[] {
  const propertyPattern = new RegExp(`\\b${property}\\s*:\\s*([^;}]+)`, "g");
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(
      ([, selector, declarations]) =>
        selector.includes(selectorPart) &&
        new RegExp(`\\b${property}\\s*:`).test(declarations),
    )
    .flatMap(([, , declarations]) =>
      [...declarations.matchAll(propertyPattern)].map((match) => match[1].trim()),
    );
}

test("places Type1 refresh immediately before the view button", () => {
  assert.match(
    htmlSource,
    /<button id="refresh"[^>]*>[^<]*<\/button>\s*<button id="pin"/,
    "refresh must stay immediately before the view button in DOM order",
  );
  // 계약은 "제 폭을 지키고, 왼쪽 자유 공간을 흡수해 Type 버튼 옆에 붙는다"이다.
  // 선언 텍스트를 통째로 고정하면 동작이 같은 리팩터링까지 빨간불이 된다.
  assert.deepEqual(
    declaredValues("#refresh", "flex"),
    ["0 0 auto"],
    "refresh must keep its own width instead of expanding into the slider",
  );
  assert.ok(
    declaredValues("#refresh", "margin-left").includes("auto") ||
      declaredValues("#refresh", "margin").some((v) => /\bauto\s*$/.test(v)),
    "refresh must absorb the free space on its left so it sits next to the view button",
  );
  assert.match(
    stylesSource,
    /body\.locked #logo,\s*body\.locked #refresh,\s*body\.locked #alpha\s*\{\s*display:\s*none;/,
    "Type2 and Type3 must keep refresh hidden",
  );
  assert.ok(
    !htmlSource.includes("tb-break") && !css.includes("tb-break") && !/\border\s*:/.test(css),
    "the two-row toolbar machinery (forced break + order assignment) is gone",
  );
});

test("keeps the icon buttons in the bottom dock, not the title bar", () => {
  const dockBlock = htmlSource.match(/<footer class="dock">([\s\S]*?)<\/footer>/);
  assert.ok(dockBlock, "the dock footer must exist");
  for (const id of ["memobtn", "tfsdbtn", "privacybtn", "clambtn", "blackbtn"]) {
    assert.ok(
      dockBlock![1].includes(`id="${id}"`),
      `#${id} must live inside the dock`,
    );
  }
  assert.match(
    dockBlock![1],
    /<div class="tb-actions dock-tray">/,
    "the tray must carry tb-actions too — button styling and hit-region reporting key off it",
  );
  assert.match(
    stylesSource,
    /body\.dock-open \.dock-tray\s*\{\s*display:\s*flex;\s*\}/,
    "the tray opens by body.dock-open",
  );
  assert.match(
    mainSource,
    /localStorage\.setItem\("switcher\.dock"/,
    "the open state must survive a restart",
  );
});

test("keeps memo visible and clickable in every view mode once the dock is open", () => {
  assert.deepEqual(
    declaredValues("#memobtn", "display"),
    ["inline-block"],
    "no view mode may hide the memo button",
  );
  assert.match(
    stylesSource,
    /body\.locked #memobtn\s*\{[\s\S]*?--locked-button-opacity:\s*0\.18;/,
    "Type2 and Type3 memo must not disappear after opacity is applied twice",
  );
  assert.deepEqual(
    declaredValues("body.minimal .dock-tray button", "padding"),
    ["1px 3px"],
    "Type3 dock buttons must keep their narrow padding",
  );
  assert.deepEqual(
    declaredValues("body.minimal .dock-tray button", "margin"),
    ["0"],
    "Type3 dock buttons must drop their side margins to fit one row",
  );
  assert.match(
    mainSource,
    /getElementById\("memobtn"\)!\.addEventListener\("click",[\s\S]*?invoke\("memo_toggle"\)/,
    "memo must keep its native toggle action",
  );
  assert.match(
    mainSource,
    /querySelectorAll<HTMLElement>\(\s*"\.tb-actions > \*, #dock-toggle, #drag-handle, \.display-row, \.collapsible",\s*\)[\s\S]*?r\.width <= 0 \|\| r\.height <= 0[\s\S]*?regions\.push/,
    "Type2 and Type3 dock buttons and handle must stay inside the native hit-region report",
  );
});

test("sizes the window around the dock", () => {
  assert.match(
    mainSource,
    /const dockHeight = dockEl\.offsetHeight;[\s\S]*?const total = tbHeight \+ content \+ dockHeight \+ 2;/,
    "the dock sits outside main, so its height must be added explicitly",
  );
  assert.match(
    mainSource,
    /titlebarEl\.offsetHeight !== tbHeight \|\| dockEl\.offsetHeight !== dockHeight/,
    "a dock height that changed mid-resize must trigger another pass",
  );
});
