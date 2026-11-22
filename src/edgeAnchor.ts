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
