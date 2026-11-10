import assert from "node:assert/strict";
import test from "node:test";
import { edgeGaugeLayout } from "../src/edgeGaugeLayout";

test("collapsed handle shrinks for missing providers without reserving blank rows", () => {
  assert.deepEqual(edgeGaugeLayout(false, false, false), { height: 0, tabHeight: 40, separatedSystem: false });
  assert.deepEqual(edgeGaugeLayout(false, false, true), { height: 19, tabHeight: 49, separatedSystem: false });
  assert.deepEqual(edgeGaugeLayout(true, false, false), { height: 19, tabHeight: 49, separatedSystem: false });
  assert.deepEqual(edgeGaugeLayout(false, true, true), { height: 54, tabHeight: 84, separatedSystem: true });
  assert.deepEqual(edgeGaugeLayout(true, false, true), { height: 54, tabHeight: 84, separatedSystem: true });
  assert.deepEqual(edgeGaugeLayout(true, true, false), { height: 44, tabHeight: 74, separatedSystem: false });
  assert.deepEqual(edgeGaugeLayout(true, true, true), { height: 79, tabHeight: 109, separatedSystem: true });
});
