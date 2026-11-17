import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ensureDist } from "../scripts/dist.mjs";

// Exercise installer decisions without downloading binaries or launching an app.
// The subprocess boundary supplies synthetic native version/extraction results.
function fixture(t: { after(fn: () => void): void }, platform = "win32") {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "switcher-dist-한글 '&%-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ version: "2.1.0" }));
  const entryName = platform === "darwin" ? "switcher.app" : "switcher.exe";
  const entry = path.join(root, "bin-dist", entryName);
  const calls: { command: string; args: string[]; options: any }[] = [];
  const downloads: string[] = [];
  const state = { version: "2.1.0", extractFails: false, downloadFails: false, onDownload: null as (() => void) | null };
  const writeApp = (target: string, version: string) => {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, version);
  };
  const run = (command: string, args: string[], options: any) => {
    calls.push({ command, args, options });
    if (command.endsWith("PlistBuddy") || command.endsWith("powershell.exe")) {
      const target = options.env?.SWITCHER_VERSION_PATH ?? path.dirname(path.dirname(args.at(-1)!));
      return { status: 0, stdout: fs.readFileSync(target, "utf8") + "\r\n" };
    }
    if (state.extractFails) return { status: 1 };
    writeApp(path.join(args.at(-1)!, entryName), state.version);
    return { status: 0 };
  };
  const fetchImpl = async (url: string) => {
    downloads.push(url);
    state.onDownload?.();
    return { ok: !state.downloadFails, arrayBuffer: async () => new ArrayBuffer(1) };
  };
  const install = () => ensureDist({ root, platform, arch: platform === "darwin" ? "arm64" : "x64", run, fetchImpl });
  return { root, entry, calls, downloads, state, install, cached: (version: string) => writeApp(entry, version) };
}

for (const platform of ["win32", "darwin"]) {
  test(`${platform}: replaces an older cached app with the exact npm version`, async t => {
    const f = fixture(t, platform);
    f.cached("1.8.5");
    assert.equal(await f.install(), f.entry);
    assert.equal(fs.readFileSync(f.entry, "utf8"), "2.1.0");
    assert.equal(f.downloads.length, 1);
    assert.match(f.downloads[0], /\/v1\.8\.5\/switcher-(win-x64|mac-arm64)-v2\.1\.0\.zip$/);
    assert.deepEqual(fs.readdirSync(f.root).sort(), ["bin-dist", "package.json"]);
  });

  test(`${platform}: keeps current and self-updated apps without a download`, async t => {
    const f = fixture(t, platform);
    for (const version of ["2.1.0", "2.10.0", "3.0.0"]) {
      f.cached(version);
      assert.equal(await f.install(), f.entry);
      assert.equal(fs.readFileSync(f.entry, "utf8"), version);
    }
    assert.deepEqual(f.downloads, []);
  });

  test(`${platform}: failed downloads/extraction and mismatched releases preserve the old app`, async t => {
    const f = fixture(t, platform);
    f.cached("1.8.5");
    f.state.downloadFails = true;
    await assert.rejects(f.install(), /다운로드/);
    f.state.downloadFails = false;
    f.state.extractFails = true;
    await assert.rejects(f.install(), /압축 해제/);
    f.state.extractFails = false;
    f.state.version = "2.0.0";
    await assert.rejects(f.install(), /일치하지 않습니다/);
    assert.equal(fs.readFileSync(f.entry, "utf8"), "1.8.5");
    assert.deepEqual(fs.readdirSync(f.root).sort(), ["bin-dist", "package.json"]);
  });

  test(`${platform}: missing and unreadable versions are repaired before launch`, async t => {
    const f = fixture(t, platform);
    assert.equal(await f.install(), f.entry);
    f.cached("not-a-version");
    assert.equal(await f.install(), f.entry);
    assert.equal(f.downloads.length, 2);
    assert.equal(fs.readFileSync(f.entry, "utf8"), "2.1.0");
  });

  test(`${platform}: does not overwrite an app upgraded while downloading`, async t => {
    const f = fixture(t, platform);
    f.cached("1.8.5");
    f.state.onDownload = () => f.cached("2.2.0");
    assert.equal(await f.install(), f.entry);
    assert.equal(fs.readFileSync(f.entry, "utf8"), "2.2.0");
    assert.deepEqual(fs.readdirSync(f.root).sort(), ["bin-dist", "package.json"]);
  });
}

test("Windows version checks pass unusual paths as data, without shell interpolation or a terminal window", async t => {
  const f = fixture(t);
  f.cached("2.1.0");
  await f.install();
  const call = f.calls[0];
  assert.equal(call.options.env.SWITCHER_VERSION_PATH, f.entry);
  assert.equal(call.options.windowsHide, true);
  assert.equal(call.options.shell, undefined);
  assert.ok(!call.args.some(arg => arg.includes(f.root)));
});
