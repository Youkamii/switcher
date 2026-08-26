import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const releaseWorkflow = readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url),
  "utf8",
);
const npmPublishWorkflow = readFileSync(
  new URL("../.github/workflows/npm-publish.yml", import.meta.url),
  "utf8",
);
const distScript = readFileSync(
  new URL("../scripts/dist.mjs", import.meta.url),
  "utf8",
);

test("builds releases only when explicitly dispatched", () => {
  assert.doesNotMatch(releaseWorkflow, /^  push:/m);
  assert.match(releaseWorkflow, /^  workflow_dispatch:/m);
});

test("uses v1.8.5 as the one permanent rolling release", () => {
  assert.match(releaseWorkflow, /^\s{6}ROLLING_RELEASE_TAG: v1\.8\.5\s*$/m);

  const createCommands =
    releaseWorkflow.match(/^\s+gh release create[^\r\n]+/gm) ?? [];
  assert.equal(createCommands.length, 1);
  assert.match(createCommands[0], /gh release create "\$ROLLING_RELEASE_TAG"/);
  assert.doesNotMatch(createCommands[0], /"\$TAG"/);

  const publishCommands =
    releaseWorkflow.match(/^\s+gh release edit[^\r\n]+--draft=false[^\r\n]*/gm) ??
    [];
  assert.equal(publishCommands.length, 1);
  assert.match(publishCommands[0], /gh release edit "\$ROLLING_RELEASE_TAG"/);
  assert.doesNotMatch(publishCommands[0], /"\$TAG"/);
});

test("serializes rolling release writes without cancelling an active publish", () => {
  assert.match(
    releaseWorkflow,
    /^  release:\r?\n[\s\S]*?^    concurrency:\r?\n^      group: switcher-rolling-release\r?\n^      cancel-in-progress: false\s*$/m,
  );
});

test("allows creating the rolling release only for the v1.8.5 bridge", () => {
  assert.match(
    releaseWorkflow,
    /if ! gh release view "\$ROLLING_RELEASE_TAG"[^\n]*; then[\s\S]*?if \[ "\$TAG" != "\$ROLLING_RELEASE_TAG" \]; then[\s\S]*?exit 1[\s\S]*?fi[\s\S]*?gh release create "\$ROLLING_RELEASE_TAG"/,
  );
  assert.match(
    releaseWorkflow,
    /RELEASE_JSON=\$\(gh release view "\$ROLLING_RELEASE_TAG" --json isDraft,assets\)[\s\S]*?IS_DRAFT=\$\(printf[^\n]*"\$RELEASE_JSON" \| jq -r \.isDraft\)[\s\S]*?if \[ "\$IS_DRAFT" = "true" \]; then\s+gh release edit "\$ROLLING_RELEASE_TAG" --draft=false\s+fi/,
  );
});

test("publishes immutable versioned assets and mutable latest aliases", () => {
  assert.match(
    releaseWorkflow,
    /WIN_VERSIONED="switcher-win-x64-\$TAG\.zip"/,
  );
  assert.match(
    releaseWorkflow,
    /MAC_VERSIONED="switcher-mac-arm64-\$TAG\.zip"/,
  );
  assert.match(releaseWorkflow, /switcher-win-x64-latest\.zip/);
  assert.match(releaseWorkflow, /switcher-mac-arm64-latest\.zip/);
  assert.match(
    releaseWorkflow,
    /gh release upload "\$ROLLING_RELEASE_TAG" "\$WIN_VERSIONED" "\$MAC_VERSIONED" --clobber/,
  );
  assert.match(
    releaseWorkflow,
    /gh release upload "\$ROLLING_RELEASE_TAG" "\$WIN_LATEST" "\$MAC_LATEST" --clobber/,
  );
});

test("keeps legacy fixed asset names frozen at the v1.8.5 bridge", () => {
  assert.match(
    releaseWorkflow,
    /if \[ "\$TAG" = "\$ROLLING_RELEASE_TAG" \]; then\s+gh release upload "\$ROLLING_RELEASE_TAG" \\\s+switcher-win-x64\.zip switcher-mac-arm64\.zip --clobber\s+fi/,
  );
});

test("distinguishes a missing manifest from a failed manifest download", () => {
  assert.match(
    releaseWorkflow,
    /HAS_MANIFEST=\$\(printf[^\n]*"\$RELEASE_JSON" \| jq -r \\\s*'any\(\.assets\[\]; \.name == "switcher-update\.json"\)'\)/,
  );
  assert.match(
    releaseWorkflow,
    /if \[ "\$HAS_MANIFEST" = "true" \]; then\s+gh release download "\$ROLLING_RELEASE_TAG"\s*\\\s*--pattern switcher-update\.json --output previous-update\.json \|\| \{[\s\S]*?exit 1\s+\}/,
  );
  assert.match(
    releaseWorkflow,
    /elif \[ "\$IS_DRAFT" != "true" \] \|\| \[ "\$TAG" != "\$ROLLING_RELEASE_TAG" \]; then\s+echo "rolling release manifest is missing"[^\n]*\s+exit 1\s+fi/,
  );

  const manifestMetadata = releaseWorkflow.search(/HAS_MANIFEST=\$\(/);
  const manifestDownload = releaseWorkflow.search(/gh release download/);
  const downloadFailure = releaseWorkflow.indexOf("exit 1", manifestDownload);
  const currentTagRead = releaseWorkflow.search(/CURRENT_TAG=\$\(jq/);

  assert.ok(manifestMetadata >= 0);
  assert.ok(manifestDownload > manifestMetadata);
  assert.ok(downloadFailure > manifestDownload);
  assert.ok(currentTagRead > downloadFailure);
});

test("rejects a version rollback before the first external asset upload", () => {
  assert.match(
    releaseWorkflow,
    /gh release download "\$ROLLING_RELEASE_TAG"\s*\\\s*--pattern switcher-update\.json --output previous-update\.json/,
  );
  assert.match(
    releaseWorkflow,
    /CURRENT_TAG=\$\(jq -er \.tag_name previous-update\.json\)/,
  );

  const tagKeyFunction =
    releaseWorkflow.match(/tag_key\(\) \{([\s\S]*?)\r?\n\s*\}/)?.[1] ?? "";
  assert.match(tagKeyFunction, /IFS=\. read -r major minor patch/);
  assert.match(
    tagKeyFunction,
    /printf[^\r\n]*"\$major"[^\r\n]*"\$minor"[^\r\n]*"\$patch"/,
  );
  assert.match(releaseWorkflow, /TAG_KEY=\$\(tag_key "\$TAG"\)/);
  assert.match(releaseWorkflow, /CURRENT_KEY=\$\(tag_key "\$CURRENT_TAG"\)/);
  assert.match(
    releaseWorkflow,
    /if \[\[ "\$TAG_KEY" < "\$CURRENT_KEY" \]\]; then/,
  );
  assert.doesNotMatch(
    releaseWorkflow,
    /if \[\[ "\$TAG" < "\$CURRENT_TAG" \]\]; then/,
  );

  const manifestDownload = releaseWorkflow.search(/gh release download/);
  const currentTagRead = releaseWorkflow.search(/CURRENT_TAG=\$\(jq/);
  const rollbackGuard = releaseWorkflow.search(
    /if \[\[ "\$TAG_KEY" < "\$CURRENT_KEY" \]\]; then/,
  );
  const rollbackExit = releaseWorkflow.indexOf("exit 1", rollbackGuard);
  const firstExternalUpload = releaseWorkflow.search(
    /^\s+gh release upload "\$ROLLING_RELEASE_TAG"/m,
  );

  assert.ok(manifestDownload >= 0);
  assert.ok(currentTagRead > manifestDownload);
  assert.ok(rollbackGuard > currentTagRead);
  assert.ok(rollbackExit > rollbackGuard);
  assert.ok(firstExternalUpload > rollbackExit);
});

test("uploads the update manifest after every binary alias", () => {
  const uploadCommands =
    releaseWorkflow.match(/^\s+gh release upload[^\r\n]+/gm) ?? [];
  assert.ok(uploadCommands.length >= 4);
  assert.match(uploadCommands.at(-1) ?? "", /switcher-update\.json/);
  assert.match(releaseWorkflow, /tag_name/);
  assert.match(releaseWorkflow, /browser_download_url/);
});

test("keeps the rolling release path connected to npm publishing", () => {
  assert.match(
    releaseWorkflow,
    /gh workflow run npm-publish\.yml --ref "\$TAG" -f tag="\$TAG"/,
  );
});

test("does not generate patch notes", () => {
  assert.doesNotMatch(releaseWorkflow, /--generate-notes/);
  assert.match(
    releaseWorkflow,
    /gh release create "\$ROLLING_RELEASE_TAG"[\s\S]*?--notes ""/,
  );
  assert.doesNotMatch(releaseWorkflow, /gh release edit[^\n]*--notes/);
});

test("npm publishing validates versioned binaries in the rolling release", () => {
  assert.match(
    npmPublishWorkflow,
    /^\s{6}ROLLING_RELEASE_TAG: v1\.8\.5\s*$/m,
  );
  assert.match(
    npmPublishWorkflow,
    /releases\/tags\/\$ROLLING_RELEASE_TAG/,
  );
  assert.doesNotMatch(
    npmPublishWorkflow,
    /releases\/tags\/\$RELEASE_TAG/,
  );
  assert.match(
    npmPublishWorkflow,
    /switcher-win-x64-\$RELEASE_TAG\.zip/,
  );
  assert.match(
    npmPublishWorkflow,
    /switcher-mac-arm64-\$RELEASE_TAG\.zip/,
  );
});

test("npm installs download their exact version from the rolling release", () => {
  assert.match(
    distScript,
    /const ROLLING_RELEASE_TAG\s*=\s*["']v1\.8\.5["'];/,
  );
  assert.match(
    distScript,
    /const versionedZip\s*=\s*`\$\{asset\.zip\.slice\(0,\s*-4\)\}-v\$\{version\}\.zip`;/,
  );
  assert.match(
    distScript,
    /releases\/download\/\$\{ROLLING_RELEASE_TAG\}\/\$\{versionedZip\}/,
  );
  assert.doesNotMatch(
    distScript,
    /releases\/download\/v\$\{version\}\/\$\{asset\.zip\}/,
  );
});
