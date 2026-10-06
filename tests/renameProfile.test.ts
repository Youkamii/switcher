import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

test("rename submit ignores repeated Enter while the first request is in flight (#177)", () => {
  assert.match(
    mainSource,
    /let submitting = false;\s*const submit = async \(\) => \{\s*if \(submitting\) return;[\s\S]*?submitting = true;\s*okBtn\.disabled = true;\s*input\.disabled = true;\s*try \{\s*await invoke\("rename_profile"/,
    "the rename row must lock both the button and the input before sending rename_profile",
  );
  assert.match(
    mainSource,
    /await invoke\("rename_profile"[\s\S]*?catch \(error\) \{\s*toast\(String\(error\), true\);\s*okBtn\.disabled = false;\s*input\.disabled = false;\s*submitting = false;/,
    "a failed rename must unlock the row so the user can fix the name and retry",
  );
});

test("rename Enter that only confirms IME composition does not submit (#177)", () => {
  assert.match(
    mainSource,
    /if \(event\.key === "Enter"\) \{[\s\S]*?if \(event\.isComposing \|\| event\.keyCode === 229\) return;\s*void submit\(\);/,
    "Korean IME confirmation Enter must not trigger rename_profile",
  );
});
