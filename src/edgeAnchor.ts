/// 엣지(벽 붙임) 손잡이의 자리 계산 (#188). 순수 함수만 둔다 — 창·저장소는 main.ts가 다룬다.
///
/// 지키는 것은 하나다: **손잡이가 벽에서 서 있는 높이**(anchor, 화면 좌표의 손잡이 중심).
/// 창은 그 손잡이를 담은 채 작업영역 안에 있으면 되고, 필요한 만큼만 움직인다. 그래서
/// - 손잡이를 끌면 창 안에서 먼저 움직이고, 창 끝에 닿으면 창이 따라온다 (벽 끝에서 끝까지 간다)
/// - 판 높이가 바뀌어도(계정·API 칸) 손잡이는 화면에서 제자리다
/// - ☰로 판을 작업영역 밖에 놓으면 창만 안으로 돌아오고 손잡이는 놓인 자리에 남는다
/// 단위는 전부 같은 논리 px(CSS px)다.

/// 손잡이와 창·작업영역 끝 사이의 여유
export const EDGE_TAB_MARGIN = 4;

/// 작업영역의 세로 구간 (창의 내용 영역이 머물 수 있는 범위)
export type EdgeWork = { top: number; height: number };

export type EdgeRange = { min: number; max: number };

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/// 손잡이 중심이 창 안에서 머물 수 있는 범위(창 위에서부터) — 손잡이 전체가 창 안에 들어온다
export function edgeTabRange(height: number, tabHeight: number): EdgeRange {
  const min = tabHeight / 2 + EDGE_TAB_MARGIN;
  return { min, max: Math.max(min, height - tabHeight / 2 - EDGE_TAB_MARGIN) };
}

/// 손잡이 중심이 벽에서 머물 수 있는 범위(화면 좌표) — 작업영역 위 끝부터 아래 끝까지
export function edgeAnchorRange(work: EdgeWork, tabHeight: number): EdgeRange {
  const min = work.top + tabHeight / 2 + EDGE_TAB_MARGIN;
  return { min, max: Math.max(min, work.top + work.height - tabHeight / 2 - EDGE_TAB_MARGIN) };
}

/// 벽에서의 높이 비율(0 = 맨 위, 1 = 맨 아래) → 화면 좌표. 저장은 비율로 한다 — 해상도·독
/// 크기가 바뀌어도 위 끝에 둔 손잡이는 위 끝에, 아래 끝에 둔 손잡이는 아래 끝에 남는다.
export function edgeAnchorAt(fraction: number, work: EdgeWork, tabHeight: number): number {
  const { min, max } = edgeAnchorRange(work, tabHeight);
  return min + clamp(fraction, 0, 1) * (max - min);
}

export function edgeAnchorFraction(anchor: number, work: EdgeWork, tabHeight: number): number {
  const { min, max } = edgeAnchorRange(work, tabHeight);
  return max > min ? clamp((anchor - min) / (max - min), 0, 1) : 0.5;
}

export type EdgePlacement = {
  /// 창(내용 영역)의 위 좌표
  top: number;
  /// 창 위에서 손잡이 중심까지
  tabY: number;
  /// 실제로 놓인 손잡이 중심 — 범위 안으로 눌린 anchor
  anchor: number;
};

/// 손잡이를 anchor에 두고 창을 놓는다. 창은 prevTop에서 가장 가까운 자리로만 움직인다.
export function edgePlacement(input: {
  anchor: number;
  height: number;
  tabHeight: number;
  work: EdgeWork;
  prevTop: number;
}): EdgePlacement {
  const { height, tabHeight, work, prevTop } = input;
  const tab = edgeTabRange(height, tabHeight);
  const wall = edgeAnchorRange(work, tabHeight);
  const anchor = clamp(input.anchor, wall.min, wall.max);
  const workLow = work.top;
  const workHigh = work.top + Math.max(0, work.height - height);
  // 창이 손잡이를 담으려면 top은 [anchor - tab.max, anchor - tab.min] 안이어야 한다
  const low = Math.max(workLow, anchor - tab.max);
  const high = Math.min(workHigh, anchor - tab.min);
  // 창이 작업영역보다 크면 겹치는 구간이 없다 — 그때는 작업영역 쪽을 지킨다
  const top = low <= high ? clamp(prevTop, low, high) : clamp(prevTop, workLow, workHigh);
  const tabY = clamp(anchor - top, tab.min, tab.max);
  return { top, tabY, anchor: top + tabY };
}

/** 판 위아래 여백 — 판(#edge-body)은 창보다 12px 안쪽에서 시작하고 끝난다 (styles.css) */
const EDGE_BODY_INSET = 12;
/** 위·아래 덩어리의 반높이 (styles.css .edge-blob:nth-child(2)·(4)의 height / 2) + 부풀 때 여유 */
const BLOB_UP_HALF = 29 + 2;
const BLOB_DOWN_HALF = 31 + 2;

export type EdgeBlobs = {
  /** 위 덩어리 중심 (창 위에서 CSS px) */
  up: number;
  /** 아래 덩어리 중심 */
  down: number;
  /** 위 덩어리를 손잡이 아래, 아래 덩어리보다 먼 자리로 옮겼는가 — 그때는 나오고 들어가는 순서도 바꾼다 */
  flipped: boolean;
};

/**
 * 액체 연출에서 손잡이 위아래로 번지는 두 덩어리의 자리 (#185 연출, 2026-11-23 사용자 보고:
 * "애니메이션 이상해졌잖아"). 손잡이 위로 판 높이의 26%, 아래로 24% — 손잡이가 가운데면 예전 그대로다.
 * 손잡이가 판 끝에 붙어 있으면(☰로 옮기면 손잡이는 판 맨 위에 선다) 한쪽 덩어리가 창 밖으로 나가
 * 잘리고 액체가 한쪽으로 쏠린 모양이 됐다. 모자란 쪽 덩어리는 반대쪽 더 먼 자리로 옮긴다 —
 * 판이 손잡이에서 아래(또는 위)로 쏟아지듯 셋이 고르게 번진다.
 */
export function edgeBlobCenters(tabY: number, height: number): EdgeBlobs {
  const reach = Math.max(0, height - EDGE_BODY_INSET * 2);
  const above = reach * 0.26;
  const below = reach * 0.24;
  const top = EDGE_BODY_INSET;
  const bottom = height - EDGE_BODY_INSET;
  let up = tabY - above;
  let down = tabY + below;
  const upFits = up - BLOB_UP_HALF >= top;
  const downFits = down + BLOB_DOWN_HALF <= bottom;
  let flipped = false;
  if (!upFits && downFits) {
    up = tabY + below + above;
    flipped = true;
  } else if (upFits && !downFits) {
    down = tabY - above - below;
  }
  const keep = (y: number, half: number) => clamp(y, top + half, Math.max(top + half, bottom - half));
  return { up: keep(up, BLOB_UP_HALF), down: keep(down, BLOB_DOWN_HALF), flipped };
}

/** 손잡이가 부풀 때의 가장 큰 세로 배율 (styles.css edge-drop-swell 22%의 scale(2.2, 1.6)) */
const TAB_SWELL_Y = 1.6;

/**
 * 손잡이가 부풀고 출렁일 때의 기준점 — 손잡이 위에서 px. 가운데에서 부풀면 판 끝에 붙은 손잡이
 * (창 위·아래 끝에서 4px)는 창 밖으로 부풀어 평평하게 잘렸다(2026-11-23 "애니메이션 이상해졌잖아").
 * 부푼 손잡이가 창 안에 남는 만큼만 기준점을 창 끝 쪽으로 옮긴다. 자리가 넉넉하면 가운데 그대로다.
 */
export function edgeTabOrigin(tabY: number, tabHeight: number, height: number): number {
  const top = tabY - tabHeight / 2;
  const grow = TAB_SWELL_Y - 1;
  const highest = top / grow;
  const lowest = (top + tabHeight * TAB_SWELL_Y - height) / grow;
  if (lowest > highest) return tabHeight / 2;
  return clamp(tabHeight / 2, lowest, highest);
}
