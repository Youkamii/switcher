/// 시스템 모니터창 — CPU·메모리·디스크·네트워크를 1초 주기로 보여주는 미니 위젯.
///
/// - 데이터는 Rust stats_read (sysinfo) — CPU·네트워크 속도는 백엔드가 지속
///   상태로 델타를 계산해 주므로 여기선 그리기만 한다.
/// - 창이 숨겨지면 폴링도 멈춘다 (visibilitychange) — 숨은 창이 CPU를 먹지 않게.
/// - 닫기(✕·ESC)는 hide만 — 창은 재사용된다 (memo창과 같은 규칙).
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface SysStats {
  cpu: number;
  mem_used: number;
  mem_total: number;
  disk_used: number;
  disk_total: number;
  net_rx: number;
  net_tx: number;
}

const monitorWindow = getCurrentWindow();

function row(id: string) {
  const root = document.getElementById(id)!;
  return {
    bar: root.querySelector<HTMLElement>(".bar")!,
    fill: root.querySelector<HTMLElement>(".bar > i")!,
    val: root.querySelector<HTMLElement>(".val")!,
  };
}
const cpu = row("row-cpu");
const mem = row("row-mem");
const dsk = row("row-dsk");
const net = row("row-net");
const beat = document.getElementById("beat")!;
const mood = document.getElementById("mood")!;

function setBar(target: ReturnType<typeof row>, percent: number) {
  const clamped = Math.max(0, Math.min(100, percent));
  target.fill.style.width = `${clamped}%`;
  target.bar.classList.toggle("hot", clamped >= 90);
}

/// 1024 단위 자동 스케일 (GB·TB) — 용량 표기용
function fmtBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1024) return `${(gb / 1024).toFixed(1)}T`;
  return `${gb.toFixed(1)}G`;
}

/// 초당 바이트 → 사람 눈에 맞는 속도 표기
function fmtRate(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${bytesPerSec}B`;
  const kb = bytesPerSec / 1024;
  if (kb < 1024) return `${Math.round(kb)}K`;
  return `${(kb / 1024).toFixed(1)}M`;
}

// ── CPU 60초 스파크라인 ──────────────────────────────────────────
const spark = document.getElementById("spark") as HTMLCanvasElement;
const HISTORY = 60;
const history: number[] = [];

function drawSpark() {
  const ctx = spark.getContext("2d");
  if (!ctx) return;
  const scale = window.devicePixelRatio || 1;
  const w = spark.clientWidth;
  const h = spark.clientHeight;
  spark.width = Math.round(w * scale);
  spark.height = Math.round(h * scale);
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (history.length < 2) return;
  const step = w / (HISTORY - 1);
  const y = (v: number) => h - 1 - (v / 100) * (h - 2);
  ctx.beginPath();
  history.forEach((v, i) => {
    const x = w - (history.length - 1 - i) * step;
    if (i === 0) ctx.moveTo(x, y(v));
    else ctx.lineTo(x, y(v));
  });
  ctx.strokeStyle = "rgba(167, 139, 250, 0.9)";
  ctx.lineWidth = 1.2;
  ctx.stroke();
  // 선 아래를 은은하게 채워 면적 그래프 느낌
  ctx.lineTo(w, h);
  ctx.lineTo(w - (history.length - 1) * step, h);
  ctx.closePath();
  ctx.fillStyle = "rgba(167, 139, 250, 0.15)";
  ctx.fill();
}

/// CPU 기분 — 한가하면 느긋, 바쁘면 진지, 뜨거우면 울상
function moodFace(cpuPct: number): string {
  if (cpuPct < 40) return "(・ᴗ・)";
  if (cpuPct < 80) return "(•̀ᴗ•́)";
  return "(>﹏<)";
}

/// 네트워크 바의 기준 — 이 세션에서 본 최고 속도 (바닥 1MB/s: 유휴가 꽉 차 보이지 않게)
let netPeak = 1024 * 1024;

let inflight = false;
async function tick() {
  if (document.visibilityState === "hidden" || inflight) return;
  inflight = true;
  try {
    const s = await invoke<SysStats>("stats_read");
    setBar(cpu, s.cpu);
    cpu.val.textContent = `${s.cpu.toFixed(1)}%`;
    history.push(s.cpu);
    if (history.length > HISTORY) history.shift();
    drawSpark();
    mood.textContent = moodFace(s.cpu);

    setBar(mem, (s.mem_used / Math.max(1, s.mem_total)) * 100);
    mem.val.textContent = `${fmtBytes(s.mem_used)}/${fmtBytes(s.mem_total)}`;

    setBar(dsk, (s.disk_used / Math.max(1, s.disk_total)) * 100);
    dsk.val.textContent = `${fmtBytes(s.disk_used)}/${fmtBytes(s.disk_total)}`;

    const flow = s.net_rx + s.net_tx;
    netPeak = Math.max(netPeak, flow);
    setBar(net, (flow / netPeak) * 100);
    net.val.textContent = `↓${fmtRate(s.net_rx)} ↑${fmtRate(s.net_tx)}`;

    // 심장박동 — 샘플이 도착했다는 신호
    beat.classList.add("pump");
    window.setTimeout(() => beat.classList.remove("pump"), 180);
  } catch {
    // 일시 실패는 다음 틱이 만회한다 — 모니터는 계속 돈다
  } finally {
    inflight = false;
  }
}

void tick();
window.setInterval(() => void tick(), 1000);
// 다시 보이게 되면 즉시 한 번 — 1초 기다리지 않고 바로 채운다
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void tick();
});

async function hideSelf() {
  await monitorWindow.hide();
}
document.getElementById("mclose")!.addEventListener("click", () => void hideSelf());
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") void hideSelf();
});
window.addEventListener("resize", drawSpark);

document.getElementById("grip")!.addEventListener("mousedown", (event) => {
  event.preventDefault();
  void monitorWindow.startResizeDragging("SouthEast");
});
