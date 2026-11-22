import assert from "node:assert/strict";
import test from "node:test";
import {
  API_SAME_GESTURE_MS,
  API_SETTLE_MS,
  cardClickAction,
  pickAccountKey,
  rememberKey,
  rememberedKey,
} from "../src/apiToggle";

const keys = [
  { name: "alpha", label: "alpha", account_id: "acct-1" },
  { name: "beta", label: "beta", account_id: "acct-1" },
  { name: "gamma", account_id: "acct-2" },
  { name: "legacy" }, // 계정에 연결되지 않은 옛 키
];

test("a repeated click uses the account's own key: last used first, never another account's", () => {
  assert.equal(pickAccountKey(keys, "acct-1")?.name, "alpha", "first key in list order by default");
  assert.equal(pickAccountKey(keys, "acct-1", "beta")?.name, "beta", "the key the account used last");
  assert.equal(pickAccountKey(keys, "acct-1", "gamma")?.name, "alpha", "a remembered key of another account is ignored");
  assert.equal(pickAccountKey(keys, "acct-1", "deleted")?.name, "alpha", "a remembered key that no longer exists is ignored");
  assert.equal(pickAccountKey(keys, "acct-2")?.name, "gamma");
  assert.equal(pickAccountKey(keys, "acct-3"), null, "an account without keys has nothing to toggle to");
  assert.equal(pickAccountKey([{ name: "legacy" }], "acct-1"), null, "unlinked legacy keys are never picked");
});

const since = (kind: "api" | "account", sincePressMs: number, sinceLandedMs = sincePressMs) => ({ kind, sincePressMs, sinceLandedMs });

test("every press toggles: account, then its API key, then back to the subscription", () => {
  const settled = since("account", 5_000);
  // 1) 다른 계정을 누르면 그 계정(구독)
  assert.deepEqual(cardClickAction({ switchable: true, active: false }, settled), { kind: "switch" });
  // 2) 사용 중인 계정을 한 번 더 누르면 그 계정의 API 키
  assert.deepEqual(cardClickAction({ switchable: false, active: true, apiKey: "alpha" }, settled), { kind: "api", key: "alpha" });
  // 3) API를 쓰는 중인 계정을 다시 누르면 구독(사용량)으로 돌아온다
  const owner = { switchable: true, active: false, apiOwner: true };
  assert.deepEqual(cardClickAction(owner, since("api", 1_000)), { kind: "switch" });
  assert.deepEqual(cardClickAction(owner, null), { kind: "switch" }, "also right after a restart, with no recorded change");
  // 키가 없는 사용 중 계정은 아무 일도 없다
  assert.equal(cardClickAction({ switchable: false, active: true }, settled), null);
  assert.equal(cardClickAction({ switchable: false, active: true, apiKey: "" }, settled), null);
  assert.equal(cardClickAction({ switchable: false, active: false, apiKey: "alpha" }, settled), null);
  // 처음 켠 직후(기록된 전환 없음)에도 한 번 더 누르면 API로 간다
  assert.deepEqual(cardClickAction({ switchable: false, active: true, apiKey: "alpha" }, null), { kind: "api", key: "alpha" });
});

test("the way back is never swallowed: a deliberate second press always returns to the subscription", () => {
  const owner = { switchable: true, active: false, apiOwner: true };
  // 화면이 바뀐 걸 보고 누른 둘째 누름은 언제든 동작한다 — 첫 누름에서 0.6초가 안 지났어도
  for (const landed of [API_SETTLE_MS, 300, 500, 599, 600, 1_500, 60_000]) {
    assert.deepEqual(cardClickAction(owner, since("api", landed + 150, landed)), { kind: "switch" }, `${landed}ms after the API showed`);
  }
  // 무시하는 건 API를 켠 바로 그 동작의 꼬리(더블클릭의 뒷누름)뿐 — 켜자마자 꺼지지 않게
  assert.equal(cardClickAction(owner, since("api", 150, 0)), null);
  assert.equal(cardClickAction(owner, since("api", 400, API_SETTLE_MS - 1)), null);
  assert.ok(API_SETTLE_MS <= 300, "the settle window must stay shorter than a human second press");
  // 다른 계정으로 가는 전환(무과금)은 쉬지 않는다
  assert.deepEqual(cardClickAction({ switchable: true, active: false }, since("account", 0)), { kind: "switch" });
  assert.deepEqual(cardClickAction({ switchable: true, active: false }, since("api", 0)), { kind: "switch" });
  assert.deepEqual(cardClickAction(owner, since("account", 0)), { kind: "switch" });
});

test("paid API use has to be a new gesture, made after the account in use was shown", () => {
  const active = { switchable: false, active: true, apiKey: "alpha" };
  const api = { kind: "api", key: "alpha" };
  for (const kind of ["account", "api"] as const) {
    // 더블클릭으로 계정을 바꾼 직후, 뒷누름이 새로 활성이 된 카드에 떨어져도 API로 넘어가지 않는다
    assert.equal(cardClickAction(active, since(kind, 120, 0)), null);
    assert.equal(cardClickAction(active, since(kind, 450, 300)), null, "inside an ordinary double-click");
    assert.equal(cardClickAction(active, since(kind, API_SAME_GESTURE_MS - 1, 400)), null);
    // 전환이 오래 걸렸으면, 끝나서 화면이 바뀐 직후의 누름도 받지 않는다 (조급한 재클릭)
    assert.equal(cardClickAction(active, since(kind, 2_100, 100)), null);
    assert.equal(cardClickAction(active, since(kind, 2_100, API_SETTLE_MS - 1)), null);
    assert.deepEqual(cardClickAction(active, since(kind, 2_100, API_SETTLE_MS)), api);
  }
  assert.ok(API_SAME_GESTURE_MS >= 500, "the gesture window must outlast an ordinary double-click");
});

test("a steady press rhythm keeps toggling: the wait is measured from the press, not from the end of the change", () => {
  const active = { switchable: false, active: true, apiKey: "alpha" };
  const owner = { switchable: true, active: false, apiOwner: true };
  // 0.7초 박자, 전환 하나에 0.3초가 걸려도: 구독 → API → 구독 → API 가 매번 넘어간다
  const beat = 700;
  const took = 300;
  assert.deepEqual(cardClickAction(owner, since("api", beat, beat - took)), { kind: "switch" });
  assert.deepEqual(cardClickAction(active, since("account", beat, beat - took)), { kind: "api", key: "alpha" });
  // 끝난 시각부터 0.6초를 쟀다면 이 누름(끝난 뒤 0.4초)은 삼켜졌다
  assert.ok(beat - took < API_SAME_GESTURE_MS);
  // 박자의 하한은 동작 구분 시간 — 그보다 빠른 연타는 같은 동작으로 본다
  assert.equal(cardClickAction(active, since("account", API_SAME_GESTURE_MS - 100, 350)), null);
  assert.deepEqual(cardClickAction(active, since("account", API_SAME_GESTURE_MS, 350)), { kind: "api", key: "alpha" });
});

test("the last used key is remembered per provider and account, by name only", () => {
  const data = new Map<string, string>();
  const store = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
  assert.equal(rememberedKey(store, "claude", "acct-1"), null);
  rememberKey(store, "claude", "acct-1", "beta");
  rememberKey(store, "codex", "acct-1", "other");
  assert.equal(rememberedKey(store, "claude", "acct-1"), "beta");
  assert.equal(rememberedKey(store, "codex", "acct-1"), "other");
  assert.equal(rememberedKey(store, "claude", "acct-2"), null);
  assert.deepEqual(JSON.parse(data.get("switcher.apikey")!), { "claude/acct-1": "beta", "codex/acct-1": "other" });
  // 손상된 저장값·쓰기 실패는 기본값(첫 키)으로 돌아갈 뿐 던지지 않는다
  data.set("switcher.apikey", "{not json");
  assert.equal(rememberedKey(store, "claude", "acct-1"), null);
  data.set("switcher.apikey", "[1,2]");
  assert.equal(rememberedKey(store, "claude", "acct-1"), null);
  const blocked = { getItem: () => null, setItem: () => { throw new Error("quota"); } };
  assert.doesNotThrow(() => rememberKey(blocked, "claude", "acct-1", "beta"));
});
