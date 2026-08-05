/// 블랙 모니터 오버레이 — 모니터 하나를 통째로 덮는 검은 캔버스.
///
/// - 창은 투명하고 캔버스가 검정을 그린다. 커서 주변에 destination-out으로
///   부드러운 구멍을 뚫으면 그 사이로 **실제 화면이 실시간으로** 비쳐 보이고,
///   매 프레임 옅은 검정을 덧칠해 안개가 다시 차오르듯 서서히 어두워진다.
/// - 해제: 마우스를 빠르게 흔들면(짧은 시간 안에 고속 방향 반전 누적) 또는 ESC.
///   해제는 Rust(black_off)가 모든 모니터의 오버레이를 한꺼번에 닫는다.
/// - 갇힘 방지(red-review): 해제 리스너를 무엇보다 먼저 등록하고, 캔버스 초기화가
///   실패하면 덮지 않고 즉시 자기 해제한다. 웹뷰가 통째로 죽는 경우까지는
///   Rust 감시 스레드의 네이티브 ESC 폴링이 최후의 해제 수단으로 받친다.
import { invoke } from "@tauri-apps/api/core";

let dismissing = false;
function dismiss() {
  if (dismissing) return;
  dismissing = true;
  void invoke("black_off");
}

// 해제 수단부터 등록 — 아래 초기화가 어떤 이유로 실패해도 ESC는 살아 있다
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") dismiss();
});

try {
  const canvas = document.getElementById("veil") as HTMLCanvasElement;
  const context = canvas.getContext("2d", { alpha: true });
  if (!context) throw new Error("2d context 없음");
  const ctx = context;

  /** 매 프레임 다시 덮는 검정의 양 — 클수록 안개가 빨리 차오른다 */
  const RESTORE_ALPHA = 0.045;
  /** 커서 구멍 반지름 (CSS px) */
  const HOLE_RADIUS = 140;
  /** 구멍 중심의 밝기(뚫는 양) — 1이면 완전히 뚫려 화면이 원래 밝기로 보인다 */
  const HOLE_STRENGTH = 0.5;

  const fitCanvas = () => {
    const scale = window.devicePixelRatio || 1;
    canvas.width = Math.round(window.innerWidth * scale);
    canvas.height = Math.round(window.innerHeight * scale);
    ctx.setTransform(scale, 0, 0, scale, 0, 0);
    // 리사이즈 직후엔 전부 검정에서 시작
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = "rgba(0, 0, 0, 1)";
    ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  };
  fitCanvas();
  window.addEventListener("resize", fitCanvas);

  // 안개 되차오름 — 옅은 검정을 매 프레임 덧칠.
  // 커서 구멍도 이벤트가 아니라 프레임 단위로 뚫는다 — 마우스 폴링레이트와 무관하게
  // 밝기가 일정하고, 커서가 멈추면 뚫지 않아 안개가 다시 차오른다 (red-review)
  let punchX = -1;
  let punchY = -1;
  let punchPending = false;
  const tick = () => {
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = `rgba(0, 0, 0, ${RESTORE_ALPHA})`;
    ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
    if (punchPending) {
      punch(punchX, punchY);
      punchPending = false;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  /// 커서 위치에 부드러운 구멍 — 중심은 많이, 가장자리는 0으로
  const punch = (x: number, y: number) => {
    ctx.globalCompositeOperation = "destination-out";
    const gradient = ctx.createRadialGradient(x, y, 0, x, y, HOLE_RADIUS);
    gradient.addColorStop(0, `rgba(0, 0, 0, ${HOLE_STRENGTH})`);
    gradient.addColorStop(1, "rgba(0, 0, 0, 0)");
    ctx.fillStyle = gradient;
    ctx.beginPath();
    ctx.arc(x, y, HOLE_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  };

  // ── 흔들기 해제 감지 ─────────────────────────────────────────────
  // 0.7초 창 안에서 고속(0.8px/ms 이상) 방향 반전이 5번 이상 쌓이면 해제.
  // 가로·세로 축을 각각 세므로 좌우·상하·원형 어느 쪽으로 휘둘러도 풀린다.
  // 일반적인 이동·작은 떨림으로는 안 풀린다.
  const SHAKE_WINDOW_MS = 700;
  const SHAKE_MIN_SPEED = 0.8;
  const SHAKE_FLIPS = 5;
  let lastX = -1;
  let lastY = -1;
  let lastT = 0;
  let lastDirX = 0;
  let lastDirY = 0;
  let flips: number[] = [];

  window.addEventListener("pointermove", (event) => {
    const now = performance.now();
    const x = event.clientX;
    const y = event.clientY;
    punchX = x;
    punchY = y;
    punchPending = true;

    if (lastX >= 0) {
      const dt = Math.max(1, now - lastT);
      const record = (velocity: number, axis: "x" | "y") => {
        const dir = Math.sign(velocity);
        if (dir === 0 || Math.abs(velocity) < SHAKE_MIN_SPEED) return;
        const prev = axis === "x" ? lastDirX : lastDirY;
        if (prev !== 0 && dir !== prev) {
          flips.push(now);
          flips = flips.filter((t) => now - t <= SHAKE_WINDOW_MS);
          if (flips.length >= SHAKE_FLIPS) dismiss();
        }
        if (axis === "x") lastDirX = dir;
        else lastDirY = dir;
      };
      record((x - lastX) / dt, "x");
      record((y - lastY) / dt, "y");
    }
    lastX = x;
    lastY = y;
    lastT = now;
  });
} catch {
  // 그릴 수 없으면 덮지도 않는다 — 검은 막에 갇히는 사고 방지
  dismiss();
}
