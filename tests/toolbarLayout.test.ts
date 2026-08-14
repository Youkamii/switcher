import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const htmlSource = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

test("places Type1 refresh immediately before the view button on the first row", () => {
  assert.match(
    htmlSource,
    /<button id="refresh"[^>]*>[^<]*<\/button>\s*<button id="pin"/,
    "refresh must stay immediately before the view button in DOM order",
  );
  assert.match(
    stylesSource,
    /#tb-break\s*\{[\s\S]*?order:\s*2;[\s\S]*?\}[\s\S]*?#refresh\s*\{\s*order:\s*0;\s*\}[\s\S]*?\.tb-actions button#refresh\s*\{\s*flex:\s*0 0 auto;\s*margin:\s*0 0 0 auto;\s*\}[\s\S]*?#pin\s*\{[\s\S]*?margin-left:\s*0;\s*\}[\s\S]*?body\.locked #pin\s*\{\s*margin-left:\s*auto;/,
    "refresh must remain on the first row without expanding into the slider",
  );
  assert.match(
    stylesSource,
    /body\.locked #logo,\s*body\.locked #refresh,\s*body\.locked #alpha\s*\{\s*display:\s*none;/,
    "Type2 and Type3 must keep refresh hidden",
  );

  const refreshOrderValues = [...stylesSource.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(([, selector, declarations]) =>
      selector.includes("#refresh") && /\border\s*:/.test(declarations),
    )
    .flatMap(([, , declarations]) =>
      [...declarations.matchAll(/\border\s*:\s*([^;}]+)/g)].map((match) =>
        match[1].trim(),
      ),
    );
  assert.deepEqual(
    refreshOrderValues,
    ["0"],
    "no later refresh rule may move it behind the forced row break",
  );
});
