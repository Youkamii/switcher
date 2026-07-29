// 의존성 설치 + 앱 빌드를 한 번에. 장황한 로그 대신 로딩 표시만 보여준다.
// 순수 Node 내장 모듈만 사용 — npm install 전에도 실행할 수 있어야 하므로.
import { spawn } from "node:child_process";

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const isTTY = process.stdout.isTTY;

function elapsed(from) {
  const s = Math.floor((Date.now() - from) / 1000);
  return s >= 60 ? `${Math.floor(s / 60)}분 ${s % 60}초` : `${s}초`;
}

// 한 단계를 로딩 표시와 함께 실행한다. 출력(stdout)은 버리고, 실패 시 stderr 끝부분만 보여준다.
function step(label, cmd, args) {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    let frame = 0;
    let timer;
    if (isTTY) {
      process.stdout.write("\x1b[?25l"); // 커서 숨김
      timer = setInterval(() => {
        process.stdout.write(`\r${FRAMES[frame++ % FRAMES.length]} ${label} … ${elapsed(started)}   `);
      }, 90);
    } else {
      process.stdout.write(`- ${label} …\n`);
    }

    const child = spawn(cmd, args, {
      stdio: ["ignore", "ignore", "pipe"],
      shell: process.platform === "win32",
    });
    let err = "";
    child.stderr.on("data", (d) => (err += d.toString()));
    child.on("close", (code) => {
      if (timer) clearInterval(timer);
      if (isTTY) process.stdout.write("\r\x1b[K");
      if (code === 0) {
        console.log(`✔ ${label} (${elapsed(started)})`);
        resolve();
      } else {
        console.error(`✗ ${label} 실패\n${err.trim().split("\n").slice(-12).join("\n")}`);
        reject(new Error(label));
      }
    });
  });
}

const t0 = Date.now();
console.log("switcher 빌드를 시작합니다.");
console.log("처음에는 Rust를 통째로 컴파일하기 때문에 5~10분 걸릴 수 있습니다 — 로딩이 멈춘 게 아니니 기다려 주세요.\n");

const isMac = process.platform === "darwin";
// 맥은 더블클릭으로 열 수 있는 .app 번들까지 만든다 (윈도우는 포터블 exe 하나면 충분)
const buildArgs = isMac
  ? ["run", "tauri", "build", "--", "--bundles", "app"]
  : ["run", "tauri", "build"];

try {
  await step("의존성 설치 중", "npm", ["install", "--no-fund", "--no-audit", "--loglevel", "error"]);
  await step("앱 빌드 중", "npm", buildArgs);
  console.log(`\n완료 (${elapsed(t0)}).`);
  console.log(
    isMac
      ? "실행 파일: src-tauri/target/release/bundle/macos/switcher.app"
      : "실행 파일: src-tauri\\target\\release\\switcher.exe",
  );
} catch {
  console.error("\n빌드에 실패했습니다. 위 메시지를 확인하세요.");
  process.exitCode = 1;
} finally {
  if (isTTY) process.stdout.write("\x1b[?25h"); // 커서 복원
}
