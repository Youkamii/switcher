import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const releaseWorkflow = readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url),
  "utf8",
);

test("publishes GitHub releases only when explicitly dispatched", () => {
  assert.doesNotMatch(releaseWorkflow, /^  push:/m);
  assert.match(releaseWorkflow, /^  workflow_dispatch:/m);
  assert.match(releaseWorkflow, /gh release create "\$TAG"/);
});

test("keeps the manual release path connected to npm publishing", () => {
  assert.match(
    releaseWorkflow,
    /gh workflow run npm-publish\.yml --ref "\$TAG" -f tag="\$TAG"/,
  );
});

test("creates GitHub releases without patch notes", () => {
  assert.doesNotMatch(releaseWorkflow, /--generate-notes/);
  assert.match(
    releaseWorkflow,
    /gh release create "\$TAG"[^\n]*--notes ""/,
  );
});
