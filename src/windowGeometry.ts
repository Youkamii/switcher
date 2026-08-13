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
