import assert from "node:assert/strict";
import test from "node:test";
import {
  cancelExactSession,
  cancelGithubBeforePrompt,
  cancelledWithCleanupWarning,
  decideFailedLogin,
} from "../src/loginLifecycle";

async function within<T>(promise: Promise<T>, milliseconds = 200): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("operation did not settle")), milliseconds);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

test("GitHub pre-prompt cancellation closes without waiting for an unresolved start", async () => {
  const start = new Promise<{ session_id: string }>(() => {});
  const calls: string[] = [];

  const result = await within(
    cancelGithubBeforePrompt({
      requestId: "request-before-prompt",
      start,
      cancelStart: async (requestId) => {
        calls.push(`cancel-start:${requestId}`);
        return true;
      },
      cancelSession: async (sessionId) => {
        calls.push(`cancel-session:${sessionId}`);
        return true;
      },
      waitForSession: async (sessionId) => {
        calls.push(`wait:${sessionId}`);
        return "unused";
      },
    }),
  );

  assert.deepEqual(result, { state: "cancelled" });
  assert.deepEqual(calls, ["cancel-start:request-before-prompt"]);
});

test("GitHub completion drains the waiter for the exact session that won cancellation", async () => {
  const calls: string[] = [];

  const result = await cancelGithubBeforePrompt({
    requestId: "request-completed",
    start: Promise.resolve({ session_id: "session-completed" }),
    cancelStart: async (requestId) => {
      calls.push(`cancel-start:${requestId}`);
      return false;
    },
    cancelSession: async (sessionId) => {
      calls.push(`cancel-session:${sessionId}`);
      return false;
    },
    waitForSession: async (sessionId) => {
      calls.push(`wait:${sessionId}`);
      return "octocat";
    },
  });

  assert.deepEqual(calls, [
    "cancel-start:request-completed",
    "cancel-session:session-completed",
    "wait:session-completed",
  ]);
  assert.deepEqual(result, {
    state: "completed",
    sessionId: "session-completed",
    completion: { status: "fulfilled", value: "octocat" },
  });
});

test("GitHub closes when the pre-prompt start already failed without leaving a session", async () => {
  let exactCancelCalled = false;
  const result = await cancelGithubBeforePrompt({
    requestId: "request-failed",
    start: Promise.reject(new Error("start failed")),
    cancelStart: async () => false,
    cancelSession: async () => {
      exactCancelCalled = true;
      return true;
    },
    waitForSession: async () => "unused",
  });

  assert.deepEqual(result, { state: "cancelled" });
  assert.equal(exactCancelCalled, false);
});

test("GitHub closes when cancellation finds no registered start promise", async () => {
  const result = await cancelGithubBeforePrompt({
    requestId: "request-without-start",
    start: null,
    cancelStart: async () => false,
    cancelSession: async () => true,
    waitForSession: async () => "unused",
  });

  assert.deepEqual(result, { state: "cancelled" });
});

test("an exact process-termination rejection propagates to the UI owner", async () => {
  await assert.rejects(
    cancelExactSession(
      "session-still-running",
      async () => {
        throw new Error("process tree is still running");
      },
      Boolean,
    ),
    /process tree is still running/,
  );
});

test("a generic cleanup warning still counts as cancelled", async () => {
  const warnings: string[] = [];
  const result = await cancelExactSession(
    "account-session",
    async () => ({ cancelled: true, cleanup_error: "temporary files remain" }),
    (outcome) => cancelledWithCleanupWarning(outcome, (message) => warnings.push(message)),
  );

  assert.equal(result, "cancelled");
  assert.deepEqual(warnings, ["temporary files remain"]);
});

test("failed login retention only keeps an exact surviving session", async () => {
  let lookups = 0;
  const unreserved = await decideFailedLogin({
    reserved: false,
    requestId: "not-reserved",
    findSession: async () => {
      lookups += 1;
      return "must-not-be-read";
    },
  });
  const cleaned = await decideFailedLogin({
    reserved: true,
    requestId: "cleaned",
    findSession: async (requestId) => {
      lookups += 1;
      assert.equal(requestId, "cleaned");
      return null;
    },
  });
  const retained = await decideFailedLogin({
    reserved: true,
    requestId: "retained",
    findSession: async (requestId) => {
      lookups += 1;
      assert.equal(requestId, "retained");
      return "session-retained";
    },
  });

  assert.deepEqual(unreserved, { action: "close" });
  assert.deepEqual(cleaned, { action: "close" });
  assert.deepEqual(retained, { action: "retain", sessionId: "session-retained" });
  assert.equal(lookups, 2);
});

test("a failed retained-session lookup propagates instead of pretending cleanup succeeded", async () => {
  await assert.rejects(
    decideFailedLogin({
      reserved: true,
      requestId: "lookup-failed",
      findSession: async () => {
        throw new Error("session lookup failed");
      },
    }),
    /session lookup failed/,
  );
});
