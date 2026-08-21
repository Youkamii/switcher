import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const htmlSource = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

/// 선언 블록을 셀렉터로 훑어 특정 속성의 값들을 모은다 — "어느 규칙에서든
/// 이 속성이 이 값 말고 다른 값으로 덮이지 않는다"를 검사하기 위한 것
function declaredValues(selectorPart: string, property: string): string[] {
  const propertyPattern = new RegExp(`\\b${property}\\s*:\\s*([^;}]+)`, "g");
  return [...stylesSource.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
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
  assert.match(
    stylesSource,
    /\.tb-actions button#refresh\s*\{\s*flex:\s*0 0 auto;\s*margin:\s*0 0 0 auto;\s*\}/,
    "refresh must keep its own width instead of expanding into the slider",
  );
  assert.match(
    stylesSource,
    /body\.locked #logo,\s*body\.locked #refresh,\s*body\.locked #alpha\s*\{\s*display:\s*none;/,
    "Type2 and Type3 must keep refresh hidden",
  );
  assert.deepEqual(
    declaredValues("#refresh", "order"),
    [],
    "the toolbar is a single row now — no rule may reorder refresh",
  );
  assert.ok(
    !htmlSource.includes("tb-break") && !stylesSource.includes("tb-break"),
    "the forced row break is gone with the two-row toolbar",
  );
});

test("keeps the icon buttons in the bottom dock, not the title bar", () => {
  const dockBlock = htmlSource.match(/<footer class="dock">([\s\S]*?)<\/footer>/);
  assert.ok(dockBlock, "the dock footer must exist");
  for (const id of ["memobtn", "tfsdbtn", "monbtn", "privacybtn", "clambtn", "blackbtn"]) {
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
  assert.match(
    stylesSource,
    /body\.minimal \.tb-actions button\s*\{\s*padding:\s*1px 3px;\s*\}[\s\S]*?body\.minimal \.tb-actions button:not\(#pin\)\s*\{\s*margin:\s*0;/,
    "Type3 buttons must keep their narrow fit rules",
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
