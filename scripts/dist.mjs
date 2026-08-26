// npm 설치본이 쓰는 빌드 내려받기 공용 모듈.
// GitHub 릴리스에서 OS에 맞는 zip을 받아 패키지 안(bin-dist/)에 풀어둔다.
// 브라우저를 거치지 않으므로 격리(quarantine)·SmartScreen 딱지가 붙지 않는다.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const DIST = path.join(ROOT, "bin-dist");
const ROLLING_RELEASE_TAG = "v1.8.5";

const ASSETS = {
  "darwin-arm64": { zip: "switcher-mac-arm64.zip", entry: "switcher.app" },
  "win32-x64": { zip: "switcher-win-x64.zip", entry: "switcher.exe" },
  // 윈도우 ARM은 x64 에뮬레이션으로 돈다
  "win32-arm64": { zip: "switcher-win-x64.zip", entry: "switcher.exe" },
};

export function platformAsset() {
  return ASSETS[`${process.platform}-${process.arch}`] ?? null;
}

/// 설치된 실행 대상 경로 (없으면 null)
export function installedEntry() {
  const asset = platformAsset();
  if (!asset) return null;
  const entry = path.join(DIST, asset.entry);
  return fs.existsSync(entry) ? entry : null;
}

async function download(url, dest) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) return false;
  fs.writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
  return true;
}

function extract(zip, dir) {
  // 맥은 ditto가 서명·심볼릭 링크를 온전히 보존한다.
  if (process.platform === "darwin") {
    return spawnSync("ditto", ["-xk", zip, dir], { stdio: "ignore" }).status === 0;
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
  return spawnSync(tar, ["-xf", zip, "-C", dir], { stdio: "ignore" }).status === 0;
}

/// 빌드가 없으면 릴리스에서 받아온다. 성공하면 실행 대상 경로를 돌려준다.
export async function ensureDist() {
  const asset = platformAsset();
  if (!asset) {
    throw new Error(
      `이 플랫폼(${process.platform}-${process.arch})용 빌드가 없습니다 — 소스 빌드(npm run setup)를 사용하세요`,
    );
  }
  const existing = installedEntry();
  if (existing) return existing;

  if (typeof fetch !== "function") {
    throw new Error("Node 18 이상이 필요합니다 — node를 업데이트하세요");
  }
  const version = JSON.parse(
    fs.readFileSync(path.join(ROOT, "package.json"), "utf8"),
  ).version;
  // npm 패키지와 정확히 같은 버전만 받는다. latest로 후퇴하면 새 패키지가
  // 구버전 실행 파일을 설치해 버전·업데이터 계약이 깨진다.
  const versionedZip = `${asset.zip.slice(0, -4)}-v${version}.zip`;
  const url = `https://github.com/Youkamii/switcher/releases/download/${ROLLING_RELEASE_TAG}/${versionedZip}`;
  fs.mkdirSync(DIST, { recursive: true });
  const tmp = path.join(os.tmpdir(), `switcher-${process.pid}-${asset.zip}`);
  try {
    if (!(await download(url, tmp))) {
      throw new Error(`v${version} 릴리스 다운로드에 실패했습니다 — 릴리스 자산과 네트워크를 확인하세요`);
    }
    if (!extract(tmp, DIST)) throw new Error("압축 해제에 실패했습니다");
  } finally {
    fs.rmSync(tmp, { force: true });
  }
  const entry = installedEntry();
  if (!entry) throw new Error("압축 해제 결과에 실행 파일이 없습니다");
  return entry;
}
