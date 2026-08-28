import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const htmlSource = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

/// 주석을 걷어낸 CSS — 셀렉터 캡처가 주석 속 ID를 규칙으로 오인하지 않게 한다.
const css = stylesSource.replace(/\/\*[\s\S]*?\*\//g, "");

/// 선언 블록을 셀렉터로 훑어 특정 속성의 값들을 모은다 — "어느 규칙에서든
/// 이 속성이 이 값 말고 다른 값으로 덮이지 않는다"를 검사하기 위한 것
function declaredValues(selectorPart: string, property: string): string[] {
  const propertyPattern = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;}]+)`, "g");
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(
      ([, selector, declarations]) =>
        selector.includes(selectorPart) &&
        new RegExp(`(?:^|;)\\s*${property}\\s*:`).test(declarations),
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
  assert.deepEqual(
    declaredValues("#refresh", "flex"),
    ["0 0 auto"],
    "refresh must keep its own width instead of expanding into the slider",
  );
  assert.ok(
    declaredValues("#refresh", "margin-left").includes("auto") ||
      declaredValues("#refresh", "margin").some((value) => /\bauto\s*$/.test(value)),
    "refresh must absorb the free space on its left so it sits next to the view button",
  );
  assert.match(
    stylesSource,
    /body\.locked #logo,\s*body\.locked #refresh,\s*body\.locked #alpha\s*\{\s*display:\s*none;/,
    "Type2 and Type3 must keep refresh hidden",
  );
  assert.ok(
    !htmlSource.includes("tb-break") &&
      !css.includes("tb-break") &&
      declaredValues(".titlebar", "order").length === 0,
    "the two-row toolbar machinery (forced break + order assignment) is gone",
  );
});

test("manual usage refresh bypasses backoff without changing automatic refresh", () => {
  assert.match(
    mainSource,
    /getElementById\("refresh"\)![\s\S]*?addEventListener\("click",[\s\S]*?render\(\{ forceUsage: true \}\)/,
    "the user refresh button must request one forced usage retry",
  );
  assert.match(
    mainSource,
    /setInterval\(\(\) => \{[\s\S]*?if \(!userIsBusy\(\)\) void render\(\);[\s\S]*?5 \* 60 \* 1000/,
    "the automatic five-minute refresh must continue to respect backoff",
  );
  assert.match(
    mainSource,
    /JSON\.stringify\(\[provider, accountId, forceRetry\]\)[\s\S]*?invoke<Usage>\("fetch_usage", \{ provider, profile, forceRetry \}\)/,
    "forced and automatic requests must not share a frontend in-flight entry",
  );
  assert.match(
    mainSource,
    /if \(opts\?\.forceUsage\) queuedForceUsage = true;[\s\S]*?thisForceUsage = thisForceUsage \|\| queuedForceUsage;/,
    "a click received during rendering must preserve the forced retry intent",
  );
  assert.match(
    mainSource,
    /retry_after_secs[\s\S]*?t\("usageRetry", \{ n: Math\.max\(1, Math\.ceil\(secs \/ 60\)\) \}\)/,
    "structured backend backoff must be shown as a localized remaining time",
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
    /<div id="dock-tray" class="tb-actions dock-tray">/,
    "the tray must carry tb-actions too — button styling and hit-region reporting key off it",
  );
  assert.match(
    dockBlock![1],
    /<button[\s\S]*?id="dock-toggle"[\s\S]*?aria-controls="dock-tray"[\s\S]*?<div id="dock-tray"/,
    "the disclosure must precede its controlled tray so Tab enters the tools after opening",
  );
  assert.match(
    mainSource,
    /dockToggle\.setAttribute\("aria-label", label\)/,
    "the icon-only disclosure must expose its translated action as an accessible name",
  );
  assert.deepEqual(
    declaredValues("#dock-toggle", "height"),
    ["24px"],
    "the dock entry point must not collapse to a hard-to-hit strip",
  );
  assert.deepEqual(
    declaredValues(".dock-tray button", "min-height"),
    ["24px"],
    "opened tool buttons must keep a usable vertical hit target",
  );
  for (const [id, key] of [
    ["blackbtn", "blackTooltip"],
    ["memobtn", "memoTooltip"],
    ["tfsdbtn", "tfsdBtnTooltip"],
    ["monbtn", "monitorTooltip"],
    ["privacybtn", "privacyTooltip"],
  ]) {
    assert.match(
      mainSource,
      new RegExp(`labelIconButton\\("${id}", t\\("${key}"\\)\\)`),
      `#${id} must expose its translated function instead of only its emoji`,
    );
  }
  assert.match(
    mainSource,
    /clamBtn\.setAttribute\("aria-label", label\)/,
    "the clamshell button must expose its current translated mode",
  );
  assert.match(
    css,
    /body:not\(\.locked\) \.dock-tray button:disabled\s*\{[\s\S]*?opacity:\s*0\.35;/,
    "a busy dock action must still look disabled in Type1",
  );
  assert.match(
    stylesSource,
    /body\.dock-open \.dock-tray\s*\{\s*display:\s*flex;\s*\}/,
    "the tray opens by body.dock-open",
  );
  assert.match(
    mainSource,
    /let dockOpen = localStorage\.getItem\("switcher\.dock"\) === "1";/,
    "the saved dock state must be restored at startup",
  );
  assert.match(
    mainSource,
    /localStorage\.setItem\("switcher\.dock"/,
    "dock changes must be persisted",
  );
  assert.match(
    mainSource,
    /dockToggle\.addEventListener\("click",[\s\S]*?applyDock\(\);[\s\S]*?\}\);\s*applyDock\(\);/,
    "the restored state must be applied after wiring the toggle",
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

test("keeps segmented bars visible at a fixed cell width", () => {
  const barBlock = css.match(/(?:^|\})\s*\.bar\s*\{([^{}]*)\}/);
  assert.ok(barBlock, "the base .bar rule must exist");
  assert.match(
    barBlock[1],
    /--seg-w:\s*8px;/,
    "every usage and SYSTEM bar must use the same absolute segment width",
  );
  assert.match(
    css,
    /transparent 0 calc\(var\(--seg-w\) - 1\.5px\)[\s\S]*?var\(--seg-w\)/,
    "segment gaps must be based on the fixed width",
  );
  assert.doesNotMatch(
    css,
    /--seg\s*:/,
    "view-specific segment counts can collapse narrow bars to zero-width fill",
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
