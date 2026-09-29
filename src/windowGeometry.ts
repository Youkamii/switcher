export type PhysicalRect = {
  position: { x: number; y: number };
  size: { width: number; height: number };
};

export type PhysicalWindow = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export function logicalWorkAreaHeight(area: PhysicalRect, scaleFactor: number): number {
  if (!Number.isFinite(scaleFactor) || scaleFactor <= 0) return area.size.height;
  return area.size.height / scaleFactor;
}

export function monitorGeometryKey(area: PhysicalRect, scaleFactor: number): string {
  return [
    area.position.x,
    area.position.y,
    area.size.width,
    area.size.height,
    scaleFactor,
  ].join(":");
}

export function clampWindowToWorkArea(
  windowRect: PhysicalWindow,
  area: PhysicalRect,
): { x: number; y: number } {
  const minX = area.position.x;
  const minY = area.position.y;
  const maxX = minX + Math.max(0, area.size.width - windowRect.width);
  const maxY = minY + Math.max(0, area.size.height - windowRect.height);
  return {
    x: Math.min(maxX, Math.max(minX, windowRect.x)),
    y: Math.min(maxY, Math.max(minY, windowRect.y)),
  };
}

export type EdgeSide = "left" | "right";

/// Type4(벽 붙임): 창 중심이 작업영역 중심보다 왼쪽이면 왼쪽 벽, 아니면 오른쪽 벽.
export function pickEdgeSide(windowRect: PhysicalWindow, area: PhysicalRect): EdgeSide {
  const windowCenter = windowRect.x + windowRect.width / 2;
  const areaCenter = area.position.x + area.size.width / 2;
  return windowCenter < areaCenter ? "left" : "right";
}

/// 창 바깥 사각형과 실제 내용(클라이언트) 영역 사이의 보이지 않는 여백 —
/// Windows의 투명 창은 그림자 자리로 좌우 8px쯤을 바깥 크기에 더 갖고 있다.
export type EdgeInsets = { left: number; right: number };

/// Type4(벽 붙임): 지정한 벽에 **보이는 내용**이 딱 닿도록 창을 놓는 좌표.
/// 바깥 여백(insets)은 벽 너머로 내보낸다 — 여백까지 안에 두면 패널이 벽에서
/// 8px 떠 보인다 (실측). 세로는 작업영역 안으로만 잡는다.
export function edgeSnapPosition(
  windowRect: PhysicalWindow,
  area: PhysicalRect,
  side: EdgeSide,
  insets: EdgeInsets = { left: 0, right: 0 },
): { x: number; y: number } {
  const x =
    side === "left"
      ? area.position.x - Math.max(0, insets.left)
      : area.position.x +
        Math.max(0, area.size.width - windowRect.width) +
        Math.max(0, insets.right);
  const { y } = clampWindowToWorkArea(windowRect, area);
  return { x, y };
}
