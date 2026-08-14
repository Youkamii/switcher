export type CancelOutcome = {
  cancelled: boolean;
  cleanup_error: string | null;
};

export type ExactCancelResult = "cancelled" | "completed";

export type GithubPrePromptCancelResult<T> =
  | { state: "cancelled" }
  | {
      state: "completed";
      sessionId: string;
      completion: PromiseSettledResult<T>;
    };

export type FailedLoginDecision =
  | { action: "close" }
  | { action: "retain"; sessionId: string };

type SessionPrompt = { session_id: string };

export function cancelledWithCleanupWarning(
  outcome: CancelOutcome,
  showWarning: (message: string) => void,
): boolean {
  if (outcome.cleanup_error) showWarning(outcome.cleanup_error);
  return outcome.cancelled;
}

export async function cancelExactSession<T>(
  sessionId: string,
  cancelSession: (sessionId: string) => Promise<T>,
  wasCancelled: (result: T) => boolean,
): Promise<ExactCancelResult> {
  const result = await cancelSession(sessionId);
  return wasCancelled(result) ? "cancelled" : "completed";
}

export async function cancelGithubBeforePrompt<T>(options: {
  requestId: string;
  start: Promise<SessionPrompt> | null;
  cancelStart: (requestId: string) => Promise<boolean>;
  cancelSession: (sessionId: string) => Promise<boolean>;
  waitForSession: (sessionId: string) => Promise<T>;
  onWait?: (wait: Promise<T>) => void;
}): Promise<GithubPrePromptCancelResult<T>> {
  if (await options.cancelStart(options.requestId)) return { state: "cancelled" };
  if (!options.start) return { state: "cancelled" };

  const [started] = await Promise.allSettled([options.start]);
  if (started.status === "rejected") return { state: "cancelled" };
  const prompt = started.value;
  const exact = await cancelExactSession(prompt.session_id, options.cancelSession, Boolean);
  if (exact === "cancelled") return { state: "cancelled" };

  const wait = options.waitForSession(prompt.session_id);
  options.onWait?.(wait);
  const [completion] = await Promise.allSettled([wait]);
  return { state: "completed", sessionId: prompt.session_id, completion };
}

export async function decideFailedLogin(options: {
  reserved: boolean;
  requestId: string;
  findSession: (requestId: string) => Promise<string | null>;
}): Promise<FailedLoginDecision> {
  if (!options.reserved) return { action: "close" };
  const sessionId = await options.findSession(options.requestId);
  return sessionId ? { action: "retain", sessionId } : { action: "close" };
}
