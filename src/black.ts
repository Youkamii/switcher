/// 블랙 모니터 오버레이 — 모니터 하나를 통째로 덮는 검은 캔버스.
///
/// - 창은 투명하고 캔버스가 검정을 그린다. 커서 주변에 destination-out으로
///   부드러운 구멍을 뚫으면 그 사이로 **실제 화면이 실시간으로** 비쳐 보이고,
///   매 프레임 옅은 검정을 덧칠해 안개가 다시 차오르듯 서서히 어두워진다.
/// - 해제: 마우스를 빠르게 흔들면(짧은 시간 안에 고속 방향 반전 누적) 또는 ESC.
///   해제는 Rust(black_off)가 모든 모니터의 오버레이를 한꺼번에 닫는다.
import { invoke } from "@tauri-apps/api/core";

const canvas = document.getElementById("veil") as HTMLCanvasElement;
const ctx = canvas.getContext("2d", { alpha: true })!;

/** 매 프레임 다시 덮는 검정의 양 — 클수록 안개가 빨리 차오른다 */
const RESTORE_ALPHA = 0.045;
/** 커서 구멍 반지름 (CSS px) */
const HOLE_RADIUS = 140;
/** 구멍 중심의 밝기(뚫는 양) — 1이면 완전히 뚫려 화면이 원래 밝기로 보인다 */
const HOLE_STRENGTH = 0.5;

function fitCanvas() {
  const scale = window.devicePixelRatio || 1;
  canvas.width = Math.round(window.innerWidth * scale);
  canvas.height = Math.round(window.innerHeight * scale);
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  // 리사이즈 직후엔 전부 검정에서 시작
  ctx.globalCompositeOperation = "source-over";
  ctx.fillStyle = "rgba(0, 0, 0, 1)";
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
}
fitCanvas();
window.addEventListener("resize", fitCanvas);

// 안개 되차오름 — 옅은 검정을 매 프레임 덧칠
function tick() {
  ctx.globalCompositeOperation = "source-over";
  ctx.fillStyle = `rgba(0, 0, 0, ${RESTORE_ALPHA})`;
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);

/// 커서 위치에 부드러운 구멍 — 중심은 많이, 가장자리는 0으로
function punch(x: number, y: number) {
  ctx.globalCompositeOperation = "destination-out";
  const gradient = ctx.createRadialGradient(x, y, 0, x, y, HOLE_RADIUS);
  gradient.addColorStop(0, `rgba(0, 0, 0, ${HOLE_STRENGTH})`);
  gradient.addColorStop(1, "rgba(0, 0, 0, 0)");
  ctx.fillStyle = gradient;
  ctx.beginPath();
  ctx.arc(x, y, HOLE_RADIUS, 0, Math.PI * 2);
  ctx.fill();
}

// ── 흔들기 해제 감지 ─────────────────────────────────────────────
// 0.7초 창 안에서 가로 방향이 고속(0.8px/ms 이상)으로 5번 이상 뒤집히면 해제.
// 일반적인 이동·작은 떨림으로는 안 풀리고, 의도적으로 좌우로 휘둘러야 풀린다.
const SHAKE_WINDOW_MS = 700;
const SHAKE_MIN_SPEED = 0.8;
const SHAKE_FLIPS = 5;
let lastX = -1;
let lastT = 0;
let lastDir = 0;
let flips: number[] = [];
let dismissing = false;

function onMove(event: PointerEvent) {
  const now = performance.now();
  const x = event.clientX;
  const y = event.clientY;
  punch(x, y);

  if (lastX >= 0) {
    const dt = Math.max(1, now - lastT);
    const vx = (x - lastX) / dt;
    const dir = Math.sign(vx);
    if (dir !== 0 && Math.abs(vx) >= SHAKE_MIN_SPEED) {
      if (lastDir !== 0 && dir !== lastDir) {
        flips.push(now);
        flips = flips.filter((t) => now - t <= SHAKE_WINDOW_MS);
        if (flips.length >= SHAKE_FLIPS && !dismissing) {
          dismissing = true;
          void invoke("black_off");
        }
      }
      lastDir = dir;
    }
  }
  lastX = x;
  lastT = now;
}
window.addEventListener("pointermove", onMove);

// 보험 해제 수단 — 흔들기 감지가 취향에 안 맞아도 항상 나갈 수 있게
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !dismissing) {
    dismissing = true;
    void invoke("black_off");
  }
});
