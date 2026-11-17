// Installed Codex, synthetic OAuth credentials, isolated home, no generation requests.
// Verifies why changing auth.json does not switch an already running session.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "switcher-codex-session-"));
fs.chmodSync(dir, 0o700);
const jwt = claims => `${Buffer.from('{"alg":"none","typ":"JWT"}').toString("base64url")}.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.synthetic-signature`;
function credentials(name) {
  const token = jwt({ sub: `fixture-${name}`, email: `${name}@example.test`, exp: 4102444800, iat: Math.floor(Date.now() / 1000),
    "https://api.openai.com/auth": { chatgpt_account_id: `fixture-${name}`, chatgpt_user_id: `user-${name}`, chatgpt_plan_type: "plus" } });
  return { auth_mode: "chatgpt", OPENAI_API_KEY: null, tokens: { id_token: token, access_token: token, refresh_token: `synthetic-refresh-${name}`, account_id: `fixture-${name}` }, last_refresh: new Date().toISOString() };
}
const alpha = credentials("alpha"), beta = credentials("beta");
const write = auth => {
  const temporary = path.join(dir, "auth.next");
  fs.writeFileSync(temporary, JSON.stringify(auth), { mode: 0o600 });
  fs.renameSync(temporary, path.join(dir, "auth.json"));
};
fs.writeFileSync(path.join(dir, "config.toml"), 'cli_auth_credentials_store = "file"\ncheck_for_update_on_startup = false\nchatgpt_base_url = "http://127.0.0.1:9"\n', { mode: 0o600 });
const env = { ...process.env, CODEX_HOME: dir };
for (const key of ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"]) delete env[key];

async function start() {
  const child = process.platform === "win32"
    ? spawn(process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", "codex app-server"], { env, cwd: dir, windowsHide: true, stdio: ["pipe", "pipe", "ignore"] })
    : spawn("codex", ["app-server"], { env, cwd: dir, stdio: ["pipe", "pipe", "ignore"] });
  let id = 0;
  const pending = new Map();
  const lines = createInterface({ input: child.stdout });
  lines.on("line", line => {
    let message; try { message = JSON.parse(line); } catch { return; }
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id); clearTimeout(request.timeout);
    if (message.error) request.reject(new Error(`Codex RPC failed (${message.error.code})`));
    else request.resolve(message.result);
  });
  const failed = () => {
    for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(new Error("Codex process exited before responding")); }
    pending.clear();
  };
  child.on("error", failed); child.on("exit", failed); child.stdin.on("error", failed);
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const requestId = ++id;
    const timeout = setTimeout(() => { pending.delete(requestId); reject(new Error(`Codex RPC timed out: ${method}`)); }, 15000);
    pending.set(requestId, { resolve, reject, timeout });
    child.stdin.write(JSON.stringify({ id: requestId, method, params }) + "\n");
  });
  const stop = async () => {
    if (child.exitCode !== null || child.signalCode !== null || !child.pid) { lines.close(); return; }
    const exited = new Promise(resolve => child.once("exit", resolve));
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
    else child.kill("SIGTERM");
    const timeout = setTimeout(() => child.kill("SIGKILL"), 3000);
    await exited; clearTimeout(timeout); lines.close();
  };
  try {
    await rpc("initialize", { clientInfo: { name: "switcher_auth_regression", version: "0.0.0" }, capabilities: { experimentalApi: true } });
    child.stdin.write('{"method":"initialized"}\n');
  } catch (error) { await stop(); throw error; }
  // This diagnostic endpoint returns only our generated fixture token. Never log it.
  return { read: () => rpc("getAuthStatus", { includeToken: true, refreshToken: false }), stop };
}

try {
  write(alpha);
  const running = await start();
  try {
    const before = await running.read();
    assert.ok(before.authToken === alpha.tokens.access_token, "initial process must read fixture A");
    write(beta);
    const after = await running.read();
    assert.ok(after.authToken === alpha.tokens.access_token, "running process still uses A after auth.json switches to B");
    console.log("PASS: existing Codex process retains account A after the file changes to B");
  } finally { await running.stop(); }
  const fresh = await start();
  try {
    const after = await fresh.read();
    assert.ok(after.authToken === beta.tokens.access_token, "new process must read fixture B");
    console.log("PASS: a newly started Codex process reads account B");
  } finally { await fresh.stop(); }
} finally {
  fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
