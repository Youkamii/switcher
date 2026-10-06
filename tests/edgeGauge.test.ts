import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const mainSource = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");

type Gauge = { provider: string; pcts: number[] };
type FakeCard = { isConnected: boolean };

/// main.ts의 commitEdgeGauge를 그대로 떼어 내 가짜 저장소·전역 게이지로 실행한다
function loadCommitEdgeGauge() {
  const source = mainSource.match(/function commitEdgeGauge\([\s\S]*?\n\}/)?.[0];
  assert.ok(source, "commitEdgeGauge must exist");
  const javascript = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const stored = new WeakMap<FakeCard, Gauge>();
  const gauge = new Map<string, number[]>();
  let applied = 0;
  const commit = Function(
    "edgeGaugeOfCard",
    "edgeGaugeByProvider",
    "applyEdgeGauge",
    `${javascript}\nreturn commitEdgeGauge;`,
  )(stored, gauge, () => {
    applied += 1;
  }) as (card: FakeCard) => void;
  return { commit, stored, gauge, appliedCount: () => applied };
}

test("a late usage response from a discarded render never overwrites the handle gauge (#177)", () => {
  const { commit, stored, gauge, appliedCount } = loadCommitEdgeGauge();
  const discarded: FakeCard = { isConnected: false };
  stored.set(discarded, { provider: "claude", pcts: [91] });

  commit(discarded);

  assert.equal(gauge.has("claude"), false);
  assert.equal(appliedCount(), 0);
});

test("a response that lands before the smooth swap is applied once the card is mounted (#177)", () => {
  const { commit, stored, gauge, appliedCount } = loadCommitEdgeGauge();
  const card: FakeCard = { isConnected: false };
  stored.set(card, { provider: "codex", pcts: [12, 40] });

  commit(card); // 응답 시점: 아직 버퍼 안
  assert.equal(gauge.has("codex"), false);

  card.isConnected = true; // render()의 app.replaceChildren(buffer)
  commit(card);
  assert.deepEqual(gauge.get("codex"), [12, 40]);
  assert.equal(appliedCount(), 1);
});

test("a mounted card without a stored gauge leaves the handle untouched", () => {
  const { commit, gauge, appliedCount } = loadCommitEdgeGauge();
  gauge.set("claude", [5]);

  commit({ isConnected: true });

  assert.deepEqual(gauge.get("claude"), [5]);
  assert.equal(appliedCount(), 0);
});

test("only the connection-guarded commit writes the provider gauge (#177)", () => {
  const writes = mainSource.match(/edgeGaugeByProvider\.set\(/g) ?? [];
  assert.equal(writes.length, 1, "edgeAccount must not write the global gauge directly");
  assert.match(
    mainSource,
    /function edgeAccount\([\s\S]*?if \(profile\.active\) \{[\s\S]*?edgeGaugeOfCard\.set\(card, \{[\s\S]*?\}\);\s*commitEdgeGauge\(card\);/,
    "the active card must stash its gauge and commit through the guard",
  );
  assert.match(
    mainSource,
    /app\.replaceChildren\(buffer\);\s*(?:\/\/[^\n]*\n\s*)*app\.querySelectorAll<HTMLElement>\("\.edge-account"\)\.forEach\(\(card\) => commitEdgeGauge\(card\)\);/,
    "render must commit gauges stashed before the swap right after installing the buffer",
  );
});
