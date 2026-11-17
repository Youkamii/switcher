// Verify the native app's actual version through the npm launcher, without launching it.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensureDist, platformAsset } from "./dist.mjs";

const repo = fileURLToPath(new URL("../", import.meta.url));
const version = JSON.parse(fs.readFileSync(path.join(repo, "package.json"), "utf8")).version;
const asset = platformAsset();
assert.ok(asset, "native packaging check supports Windows/macOS");
const source = path.join(repo, "src-tauri", "target", "release", ...(process.platform === "darwin" ? ["bundle", "macos"] : []), asset.entry);
assert.ok(fs.existsSync(source), "build the native app before checking its npm entry");
const root = fs.mkdtempSync(path.join(os.tmpdir(), "switcher-native-dist-한글 '&%-"));
try {
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ version }));
  const entry = path.join(root, "bin-dist", asset.entry);
  fs.mkdirSync(path.dirname(entry));
  fs.cpSync(source, entry, { recursive: true });
  assert.equal(await ensureDist({ root, fetchImpl: () => { throw new Error("native version inspection failed: unexpected download"); } }), entry);
  console.log(`PASS: ${process.platform} npm launcher recognizes the native v${version} build`);
} finally {
  fs.rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
