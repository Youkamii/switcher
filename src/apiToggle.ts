/** Widget-mode account ↔ API toggle rules. Pure, so every guard can be tested without a WebView. */

export type KeyRef = { name: string; label?: string | null; account_id?: string | null };

/**
 * The key a repeated click on an account activates: the key that account used last,
 * when it still exists, otherwise its first key in list order.
 * Keys of other accounts and unlinked legacy keys are never chosen.
 */
export function pickAccountKey(keys: readonly KeyRef[], accountId: string, remembered?: string | null): KeyRef | null {
  const owned = keys.filter((key) => key.account_id === accountId);
  if (owned.length === 0) return null;
  return owned.find((key) => key.name === remembered) ?? owned[0];
}

export type CardState = {
  /** Another account, or the owner of the API in use: a press selects its subscription. */
  switchable: boolean;
  /** The subscription account currently in use. */
  active: boolean;
  /** Stored key of that account, when it has one. */
  apiKey?: string | null;
  /** The account whose API key is in use right now (state 2). */
  apiOwner?: boolean;
};
export type CardAction = { kind: "switch" } | { kind: "api"; key: string } | null;
/** The last account or API change, measured at the moment of the press being judged. */
export type LastChange = {
  kind: "api" | "account";
  /** Since the press that started it. A change made elsewhere counts from when it landed. */
  sincePressMs: number;
  /** Since it landed, i.e. how long the new state has been on screen. */
  sinceLandedMs: number;
} | null;

/**
 * A press this soon after the press that made the last change is the tail of the same gesture
 * (the second half of a double-click), not a new decision. It is measured from the press, not
 * from when the change finished, so a deliberate second press is never held back by however
 * long the change took. Click counts are not used: the system double-click interval can be
 * seconds long, and that would swallow deliberate presses.
 */
export const API_SAME_GESTURE_MS = 600;
/** No press can be a response to a state that has been on screen for less than this. */
export const API_SETTLE_MS = 250;

/**
 * Every press toggles: one press = use the account, one more press on the account in use =
 * use its API key, and a press on the account whose API is in use = back to its subscription.
 */
export function cardClickAction(card: CardState, last: LastChange): CardAction {
  const settled = !last || last.sinceLandedMs >= API_SETTLE_MS;
  if (card.switchable) {
    // The way back to the subscription is free and always open. Only the tail of the very
    // gesture that selected the API is ignored, so a double-click cannot undo what it just did.
    if (card.apiOwner && last?.kind === "api" && !settled) return null;
    return { kind: "switch" };
  }
  if (card.active && card.apiKey) {
    // The API is billed separately, so it has to be a second decision: a new gesture, made
    // after the account in use was shown.
    const newGesture = !last || last.sincePressMs >= API_SAME_GESTURE_MS;
    return settled && newGesture ? { kind: "api", key: card.apiKey } : null;
  }
  return null;
}

const MEMORY_KEY = "switcher.apikey";
type Store = Pick<Storage, "getItem" | "setItem">;

function memory(store: Store): Record<string, string> {
  try {
    const parsed: unknown = JSON.parse(store.getItem(MEMORY_KEY) ?? "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as Record<string, string>) : {};
  } catch {
    return {};
  }
}

/** Last key name used by an account. Names only: secrets never leave the OS store. */
export function rememberedKey(store: Store, provider: string, accountId: string): string | null {
  const name = memory(store)[`${provider}/${accountId}`];
  return typeof name === "string" ? name : null;
}

export function rememberKey(store: Store, provider: string, accountId: string, name: string): void {
  const all = memory(store);
  const id = `${provider}/${accountId}`;
  if (all[id] === name) return;
  all[id] = name;
  try {
    store.setItem(MEMORY_KEY, JSON.stringify(all));
  } catch {
    // A full or blocked store only loses the preference; the first key is used instead.
  }
}
