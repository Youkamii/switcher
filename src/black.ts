/// 블랙 모니터 오버레이 — 모니터 하나를 통째로 덮는 검은 캔버스.
///
/// - 창은 투명하고 캔버스가 검정을 그린다. 커서 주변에 destination-out으로
///   부드러운 구멍을 뚫으면 그 사이로 **실제 화면이 실시간으로** 비쳐 보이고,
///   매 프레임 연기 얼룩 타일 두 겹을 흘려보내듯 덧칠해 연기가 다시
///   차오르듯 얼룩덜룩하게 어두워진다.
/// - 해제: 마우스를 1~2초 계속 세게 흔들거나 ESC. 마지막 커서 위치에서 빛이
///   퍼지는 연출 뒤 Rust(black_off)가 모든 모니터의 오버레이를 한꺼번에 닫는다.
/// - 갇힘 방지(red-review): 해제 리스너를 무엇보다 먼저 등록하고, 캔버스 초기화가
///   실패하면 덮지 않고 즉시 자기 해제한다. 웹뷰가 통째로 죽는 경우까지는
///   Rust 감시 스레드의 네이티브 ESC 폴링이 최후의 해제 수단으로 받친다.
import { invoke } from "@tauri-apps/api/core";

let dismissing = false;
/// 해제 절차 — 캔버스가 살아 있으면 아래에서 "퍼지는 빛" 연출로 교체된다.
/// 연출 없이도(초기화 실패 등) 반드시 닫히는 게 우선이다.
let reveal: (() => void) | null = null;
function dismiss() {
  if (dismissing) return;
  dismissing = true;
  if (reveal) reveal();
  else void invoke("black_off");
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

  /** 매 프레임 균일하게 덮는 최소 검정 — 연기 층이 그 위에 얹힌다 */
  const BASE_RESTORE = 0.008;
  /** 연기 층 한 겹의 세기 — 타일 얼룩 밀도와 곱해져 되차오름 속도가 된다 */
  const SMOKE_ALPHA = 0.045;
  /** 커서 구멍 반지름 (CSS px) */
  const HOLE_RADIUS = 140;
  /** 구멍 중심의 밝기(뚫는 양) — 1이면 완전히 뚫려 화면이 원래 밝기로 보인다 */
  const HOLE_STRENGTH = 0.5;

  // ── 연기 질감 (사용자 요청): 무작위 뭉게 얼룩 타일을 한 번 그려두고, 매
  // 프레임 두 겹을 서로 다른 방향·속도·배율로 흘려보내며 덮는다 — 균일한
  // 되차오름 대신 얼룩덜룩하게 삼켜져 연기처럼 보인다. 얼룩은 ±타일 크기로
  // 반복해 그려 이어 붙여도 경계가 없다.
  const TILE = 512;
  const smokeTile = document.createElement("canvas");
  smokeTile.width = TILE;
  smokeTile.height = TILE;
  const smokePattern = (() => {
    const tile = smokeTile.getContext("2d");
    if (!tile) return null;
    // 얼룩 뿌리기 — 이어 붙여도 경계가 없게 ±타일 크기로 반복해 그린다
    const scatter = (count: number, minR: number, maxR: number, minA: number, maxA: number) => {
      for (let i = 0; i < count; i++) {
        const blobX = Math.random() * TILE;
        const blobY = Math.random() * TILE;
        const blobR = minR + Math.random() * (maxR - minR);
        const blobA = minA + Math.random() * (maxA - minA);
        for (const offX of [-TILE, 0, TILE]) {
          for (const offY of [-TILE, 0, TILE]) {
            const x = blobX + offX;
            const y = blobY + offY;
            const g = tile.createRadialGradient(x, y, 0, x, y, blobR);
            g.addColorStop(0, `rgba(0, 0, 0, ${blobA})`);
            g.addColorStop(1, "rgba(0, 0, 0, 0)");
            tile.fillStyle = g;
            tile.beginPath();
            tile.arc(x, y, blobR, 0, Math.PI * 2);
            tile.fill();
          }
        }
      }
    };
    scatter(40, 60, 170, 0.15, 0.5); // 큰 뭉게 — 형태
    scatter(50, 15, 60, 0.2, 0.6); //   잔 얼룩 — 질감
    return ctx.createPattern(smokeTile, "repeat");
  })();

  /// 연기 한 겹 — 패턴을 (shiftX, shiftY)만큼 흘려보내며 얇게 덮는다
  const drawSmokeLayer = (
    shiftX: number,
    shiftY: number,
    alpha: number,
    scale: number,
  ) => {
    if (!smokePattern) return;
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.scale(scale, scale);
    ctx.translate(shiftX, shiftY);
    ctx.fillStyle = smokePattern;
    ctx.fillRect(
      -shiftX - TILE,
      -shiftY - TILE,
      window.innerWidth / scale + TILE * 2,
      window.innerHeight / scale + TILE * 2,
    );
    ctx.restore();
  };

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
  // 커서 구멍은 프레임 단위로 뚫되, 프레임 사이에 지나간 경로를 따라 일정
  // 간격으로 이어 찍는다 — 빠르게 움직여도 궤적이 끊기지 않는다 (사용자 피드백).
  // 커서가 멈추면 뚫지 않아 안개가 다시 차오른다 (red-review)
  /** 궤적 보간 간격 — 구멍 반지름 대비 촘촘함 */
  const STAMP_SPACING = HOLE_RADIUS * 0.35;
  /** 이 이상 점프(모니터 간 이동 등)는 선으로 잇지 않고 새로 시작한다 */
  const TELEPORT_PX = 500;
  let pending: { x: number; y: number }[] = [];
  let lastStamp: { x: number; y: number } | null = null;
  const tick = () => {
    if (dismissing) return; // 해제 연출이 캔버스를 넘겨받는다
    ctx.globalCompositeOperation = "source-over";
    // 연기 타일을 못 만들었으면 균일 되차오름으로 대체 (기능은 유지)
    ctx.fillStyle = `rgba(0, 0, 0, ${smokePattern ? BASE_RESTORE : 0.045})`;
    ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
    const now = performance.now();
    // 세 겹 연기 — 큰 뭉게는 느리게, 잔결은 빠르게. 전체적으로 위로 살짝
    // 떠오르고, 사인 흔들림을 섞어 직선으로 흐르지 않게 한다.
    const sway = Math.sin(now * 0.00013) * 40;
    drawSmokeLayer(
      (now * 0.008 + sway) % TILE,
      (-now * 0.005) % TILE,
      SMOKE_ALPHA * 0.9,
      2.6,
    );
    drawSmokeLayer(
      (-now * 0.013 - sway) % TILE,
      (-now * 0.008) % TILE,
      SMOKE_ALPHA,
      1.3,
    );
    drawSmokeLayer(
      (now * 0.024) % TILE,
      (-now * 0.014 + sway) % TILE,
      SMOKE_ALPHA * 0.5,
      0.65,
    );
    for (const point of pending) {
      const from = lastStamp;
      lastStamp = point;
      if (!from) {
        punchSmoky(point.x, point.y);
        continue;
      }
      const dist = Math.hypot(point.x - from.x, point.y - from.y);
      if (dist > TELEPORT_PX) {
        punchSmoky(point.x, point.y);
        continue;
      }
      for (let d = STAMP_SPACING; d < dist; d += STAMP_SPACING) {
        const f = d / dist;
        punchSmoky(
          from.x + (point.x - from.x) * f,
          from.y + (point.y - from.y) * f,
        );
      }
      punchSmoky(point.x, point.y);
    }
    pending = [];
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  /// 부드러운 구멍 — 중심은 많이, 가장자리는 0으로
  const punch = (
    x: number,
    y: number,
    radius = HOLE_RADIUS,
    strength = HOLE_STRENGTH,
  ) => {
    ctx.globalCompositeOperation = "destination-out";
    const gradient = ctx.createRadialGradient(x, y, 0, x, y, radius);
    gradient.addColorStop(0, `rgba(0, 0, 0, ${strength})`);
    gradient.addColorStop(1, "rgba(0, 0, 0, 0)");
    ctx.fillStyle = gradient;
    ctx.beginPath();
    ctx.arc(x, y, radius, 0, Math.PI * 2);
    ctx.fill();
  };

  /// 커서 구멍 — 본 구멍 주위에 잔구멍 두 개를 무작위로 흩뿌려
  /// 가장자리를 연기 걷히듯 흐트러뜨린다
  const punchSmoky = (x: number, y: number) => {
    punch(x, y);
    for (let i = 0; i < 2; i++) {
      const angle = Math.random() * Math.PI * 2;
      const dist = HOLE_RADIUS * (0.2 + Math.random() * 0.35);
      punch(
        x + Math.cos(angle) * dist,
        y + Math.sin(angle) * dist,
        HOLE_RADIUS * (0.4 + Math.random() * 0.35),
        HOLE_STRENGTH * 0.35,
      );
    }
  };

  // ── 흔들기 해제 감지 ─────────────────────────────────────────────
  // 2초 창 안에서 고속(0.8px/ms 이상) 방향 반전이 12번 이상 쌓이면 해제 —
  // 대략 1~2초 동안 계속 세게 흔들어야 풀린다. 가로·세로 축을 각각 세므로
  // 좌우·상하·원형 어느 쪽으로 휘둘러도 되고, 일반적인 이동·짧은 슥슥으로는
  // 안 풀린다 (사용자 피드백 2회 반영: 가볍게 움직여도 풀려버림 → 더 오래).
  // 주의: 이 12/0.8은 사용자가 잡은 기준값이다 — v1.7.19에서 임의로 8/0.7로
  // 낮췄다가 지시한 적 없다는 질책을 받고 복구했다. "늦게 풀린다" 류의 보고는
  // 판정 기준이 아니라 입력 전달(포커스·이벤트) 문제부터 의심할 것.
  const SHAKE_WINDOW_MS = 2000;
  const SHAKE_MIN_SPEED = 0.8;
  const SHAKE_FLIPS = 12;
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
    if (!dismissing) pending.push({ x, y });

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

  // ── 해제 연출: 마지막 커서 위치에서 빛이 퍼지듯 밝아진 뒤 닫는다 ──
  // 이 오버레이(해제를 감지한 모니터)에서만 그리고, 연출이 끝나면 black_off가
  // 모든 모니터를 닫는다. 어떤 이유로든 연출이 못 끝나면 타임아웃이 닫는다.
  const DISMISS_MS = 420;
  reveal = () => {
    window.setTimeout(() => void invoke("black_off"), DISMISS_MS + 400);
    const cx = lastX >= 0 ? lastX : window.innerWidth / 2;
    const cy = lastY >= 0 ? lastY : window.innerHeight / 2;
    const maxR = Math.hypot(
      Math.max(cx, window.innerWidth - cx),
      Math.max(cy, window.innerHeight - cy),
    );
    const start = performance.now();
    const step = () => {
      const t = Math.min(1, (performance.now() - start) / DISMISS_MS);
      const eased = 1 - (1 - t) ** 3;
      // 구멍 크기에서 시작해 가장 먼 모서리 너머까지 — 끝나면 오버레이가 닫힌다
      const radius = HOLE_RADIUS + maxR * eased;
      ctx.globalCompositeOperation = "destination-out";
      const gradient = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius);
      gradient.addColorStop(0, "rgba(0, 0, 0, 1)");
      gradient.addColorStop(0.7, "rgba(0, 0, 0, 0.9)");
      gradient.addColorStop(1, "rgba(0, 0, 0, 0)");
      ctx.fillStyle = gradient;
      ctx.beginPath();
      ctx.arc(cx, cy, radius, 0, Math.PI * 2);
      ctx.fill();
      if (t < 1) requestAnimationFrame(step);
      else void invoke("black_off");
    };
    requestAnimationFrame(step);
  };
} catch {
  // 그릴 수 없으면 덮지도 않는다 — 검은 막에 갇히는 사고 방지
  dismiss();
}
