// npm 설치본이 쓰는 빌드 내려받기 공용 모듈.
// GitHub 릴리스에서 OS에 맞는 zip을 받아 패키지 안(bin-dist/)에 풀어둔다.
// 브라우저를 거치지 않으므로 격리(quarantine)·SmartScreen 딱지가 붙지 않는다.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const ROLLING_RELEASE_TAG = "v1.8.5";

const ASSETS = {
  "darwin-arm64": { zip: "switcher-mac-arm64.zip", entry: "switcher.app" },
  "win32-x64": { zip: "switcher-win-x64.zip", entry: "switcher.exe" },
  // 윈도우 ARM은 x64 에뮬레이션으로 돈다
  "win32-arm64": { zip: "switcher-win-x64.zip", entry: "switcher.exe" },
};

export function platformAsset(platform = process.platform, arch = process.arch) {
  return ASSETS[`${platform}-${arch}`] ?? null;
}

/// 설치된 실행 대상 경로 (없으면 null)
export function installedEntry(root = ROOT, asset = platformAsset()) {
  if (!asset) return null;
  const entry = path.join(root, "bin-dist", asset.entry);
  return fs.existsSync(entry) ? entry : null;
}

async function download(url, dest, fetchImpl) {
  const res = await fetchImpl(url, { redirect: "follow" });
  if (!res.ok) return false;
  fs.writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
  return true;
}

function extract(zip, dir, platform, run) {
  // 맥은 ditto가 서명·심볼릭 링크를 온전히 보존한다.
  if (platform === "darwin") {
    return run("ditto", ["-xk", zip, dir], { stdio: "ignore" }).status === 0;
  }
  // 윈도우는 내장 tar(bsdtar)가 zip을 푼다. 단 PATH의 "tar"는 Git Bash에서
  // GNU tar(zip 해제 불가)로 잡혀 첫 실행이 죽는다 — System32의 bsdtar를
  // 절대 경로로 지정해 어느 셸에서 실행해도 같은 도구를 쓴다.
  const sysTar = path.join(
    process.env.SystemRoot ?? "C:\\Windows",
    "System32",
    "tar.exe",
  );
  const tar = fs.existsSync(sysTar) ? sysTar : "tar";
  return run(tar, ["-xf", zip, "-C", dir], { stdio: "ignore", windowsHide: true }).status === 0;
}

function versionParts(value) {
  if (typeof value !== "string" || !/^\d+\.\d+\.\d+$/.test(value.trim())) return null;
  const parts = value.trim().split(".").map(Number);
  return parts.every(Number.isSafeInteger) ? parts : null;
}

function compareVersions(a, b) {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  return 0;
}

// Inspect the application itself: an extraction marker can survive an in-app update.
function binaryVersion(entry, platform, run) {
  let result;
  if (platform === "darwin") {
    result = run("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleShortVersionString", path.join(entry, "Contents", "Info.plist")], { encoding: "utf8", timeout: 10000 });
  } else {
    const powershell = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
    result = run(powershell, ["-NoProfile", "-NonInteractive", "-Command", '$v = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($env:SWITCHER_VERSION_PATH); "{0}.{1}.{2}" -f $v.FileMajorPart,$v.FileMinorPart,$v.FileBuildPart'], {
      encoding: "utf8", timeout: 10000, windowsHide: true,
      // Keep spaces, quotes and shell metacharacters out of PowerShell source.
      env: { ...process.env, SWITCHER_VERSION_PATH: entry },
    });
  }
  return result.status === 0 ? versionParts(result.stdout ?? "") : null;
}

/// Keep a current/newer app; repair stale or incomplete cached downloads before launch.
export async function ensureDist({ root = ROOT, platform = process.platform, arch = process.arch, run = spawnSync, fetchImpl = globalThis.fetch } = {}) {
  const asset = platformAsset(platform, arch);
  if (!asset) {
    throw new Error(
      `이 플랫폼(${platform}-${arch})용 빌드가 없습니다 — 소스 빌드(npm run setup)를 사용하세요`,
    );
  }
  const version = JSON.parse(
    fs.readFileSync(path.join(root, "package.json"), "utf8"),
  ).version;
  const expected = versionParts(version);
  if (!expected) throw new Error("npm 패키지 버전을 확인하지 못했습니다");
  const existing = installedEntry(root, asset);
  const current = existing ? binaryVersion(existing, platform, run) : null;
  // The app updater may already have installed a newer release than the npm wrapper.
  if (current && compareVersions(current, expected) >= 0) return existing;

  if (typeof fetchImpl !== "function") {
    throw new Error("Node 18 이상이 필요합니다 — node를 업데이트하세요");
  }
  // npm 패키지와 정확히 같은 버전만 받는다. latest로 후퇴하면 새 패키지가
  // 구버전 실행 파일을 설치해 버전·업데이터 계약이 깨진다.
  const versionedZip = `${asset.zip.slice(0, -4)}-v${version}.zip`;
  const url = `https://github.com/Youkamii/switcher/releases/download/${ROLLING_RELEASE_TAG}/${versionedZip}`;
  const dist = path.join(root, "bin-dist");
  const work = fs.mkdtempSync(path.join(root, ".switcher-download-"));
  const tmp = path.join(work, asset.zip);
  const staged = path.join(work, "bin-dist");
  const backup = path.join(work, "previous");
  let backedUp = false;
  let installed = false;
  try {
    if (!(await download(url, tmp, fetchImpl))) {
      throw new Error(`v${version} 릴리스 다운로드에 실패했습니다 — 릴리스 자산과 네트워크를 확인하세요`);
    }
    fs.mkdirSync(staged);
    if (!extract(tmp, staged, platform, run)) throw new Error("압축 해제에 실패했습니다");
    const entry = path.join(staged, asset.entry);
    const received = fs.existsSync(entry) ? binaryVersion(entry, platform, run) : null;
    if (!received || compareVersions(received, expected) !== 0) {
      throw new Error(`내려받은 앱이 v${version}과 일치하지 않습니다 — 기존 앱은 보존했습니다`);
    }
    // Another launcher or the in-app updater may finish while the download awaits.
    const latestEntry = installedEntry(root, asset);
    const latestVersion = latestEntry ? binaryVersion(latestEntry, platform, run) : null;
    if (latestVersion && compareVersions(latestVersion, expected) >= 0) return latestEntry;
    try {
      if (fs.existsSync(dist)) { fs.renameSync(dist, backup); backedUp = true; }
      fs.renameSync(staged, dist);
      installed = true;
    } catch {
      if (backedUp && !fs.existsSync(dist)) { fs.renameSync(backup, dist); backedUp = false; }
      throw new Error("앱 파일을 교체하지 못했습니다 — Switcher를 트레이에서 종료한 뒤 다시 실행하세요");
    }
  } finally {
    // Never remove the only backup if restoring it failed. A running Windows app
    // may also keep old files locked; cleanup failure must not undo a valid install.
    if (!backedUp || installed) {
      try { fs.rmSync(work, { recursive: true, force: true }); } catch { /* locked old app */ }
    }
  }
  const entry = installedEntry(root, asset);
  if (!entry) throw new Error("압축 해제 결과에 실행 파일이 없습니다");
  return entry;
}
