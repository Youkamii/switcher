# 개선 적용 현황

작업 브랜치: `feat/connection-workspace`. 코드에 적용하고 로컬 커밋으로 보관한다. 배포·푸시는 하지 않았다.

| 작업 | 이슈 | 이번 적용 | 남은 범위 |
| --- | --- | --- | --- |
| UI 재설계·레이아웃 | [#179](https://github.com/Youkamii/switcher/issues/179) | 관리 420px / 위젯 280px, 중립 배경·민트 기본색, 고정 열의 액션 버튼, 재연결 버튼, 숨긴 AI 행과 구분선 제거, 손잡이 높이 동기화, 오류 표시 | 네이티브 macOS 상호작용 및 Windows 화면 검증. 별도 관리 창을 만들지 않고 기존 4개 보기를 유지 |
| 인증·재연결 | [#180](https://github.com/Youkamii/switcher/issues/180) | invalid_grant와 권한/요청 오류 구분, stale 수치에도 실패 이유, 작은 위젯 오류 표식, Codex 사용자 경로 및 저장소 검사, 카드 재연결 | 외부 CLI와의 토큰 갱신 경합, 네이티브 키링 어댑터, 진단 내보내기. 실제 만료 자체가 사라졌다고 주장하지 않음 |
| API 연결 | [#181](https://github.com/Youkamii/switcher/issues/181) | 등록·교체·삭제·모델 목록 확인·실행 명령 복사, macOS 키체인/Windows DPAPI, 별도 CLI 설정·기록, 자동 과금 전환 없음 | 실 CLI/API 계정 검증, Windows 런처 실행 검증, 새 UI 문구의 한·영 외 번역. API 비용/쿼터는 표시하지 않음 |
| 배포 서명 | [#182](https://github.com/Youkamii/switcher/issues/182) | 위험·완료 기준 이슈화 | 미적용. 배포 신뢰키·릴리스 서명 절차와 설치본 전환 정책을 함께 준비해야 함 |

[개선 계획](IMPROVEMENT_PLAN.md) · [기준선](REVIEW_TEST_RESULTS.md) · [API 사용법](API_CONNECTIONS.md) · [시안 HTML](design/index.html) · [네 모드 구현 화면](design/four-modes.png)

화면 이미지는 실제 프로덕션 프론트 코드에 가짜 계정을 주입한 결과다. 실계정 화면이 아니며, 네이티브 WKWebView 검증을 대신하지 않는다. 시안 HTML에는 예시 데이터만 있고 실제 키 입력은 비활성화돼 있다.

## 검증 결과 (2026-11-10)

| 검사 | 결과 |
| --- | --- |
| `npm test` | 93 통과 |
| `npm run build` | TypeScript/Vite 성공 |
| `cargo test --lib` | 221 통과 / 16 ignored (실계정 e2e 포함, 미실행) |
| `cargo test api_connections -- --include-ignored` | 5 통과. 가짜 키로 실제 macOS 키체인 왕복·삭제 포함 |
| `node scripts/review-layout.mjs` | 8/8 통과. 수정 전 4/8 실패 → 0 실패 |
| `node scripts/test-workspace-ui.mjs --all-modes docs/design` | 55/55 통과. 실제 번들 + 가짜 Tauri 경계, 클립보드 대기에도 렌더 진행·등록·갱신 중 입력 유지·입력 삭제·확인·삭제 확인·숨김·오류·배치 검증 |
| macOS 앱 번들 | 빌드 성공. ad-hoc 서명, notarization 없음 |
| Windows 전체 `cargo check` | macOS 환경에 Windows SDK 헤더(`assert.h`)가 없어 ring 컴파일 단계 중단. 제품 오류로 판정하지 않음 |
| DPAPI 함수 Windows 타깃 타입 검사 | 성공. 별도 최소 크레이트로 검사. Windows 실제 암복호화·PowerShell 실행은 미검증 |

기존 Rust 경고 4개는 남아 있다. 실제 사용자 프로필·OAuth 토큰·API 키를 읽거나 변경하는 실계정 e2e와 유료 생성 요청은 실행하지 않았다. 키체인 테스트가 만든 고유한 가짜 항목은 삭제까지 확인했다. 실행 중인 설치본은 교체하지 않았다.

## 커밋

- `9963d46` 개선 계획·시안·수정 전 레이아웃 검사
- `b18c7c0` 숨긴 AI 공간·호버 카드 이동 수정
- `b0c0b51` 인증 오류 분류·실패 이유 보존
- `9f21ce4` Codex 경로·인증 저장소 보호
- `672474e` API 연결·관리 화면·회귀 검증

## 버튼·4개 모드 재검증 (2026-11-10 후속)

앞선 화면에서는 활성·비활성 계정의 자동 여백 때문에 버튼 열이 어긋났고, 접힌 도구 독으로 기능을 찾기 어려웠다. 네 모드를 함께 검증하지 않은 상태에서 화면을 제시한 점을 수정했다.

- 전환/사용 중 · 다시 연결 · 이름 · 삭제를 고정 열로 배치. 이름·삭제를 더보기 메뉴에서 꺼냈다.
- 일반 화면에 4개 모드 직접 선택 버튼 추가. 기존 일반·컴팩트·미니멀·엣지 동작을 모두 유지.
- 메모·자동 전환·가리기·잠자기 방지·화면 끄기 도구를 기본 펼침으로 변경. 작은 모드의 낮은 버튼 불투명도와 미니멀의 가리기 숨김 규칙 제거. 사용자가 직접 접는 선택은 저장.
- 도구 아이콘을 SVG로 통일. 일반 모드에는 이름도 표시하며, 긴 화면에는 스크롤 표시를 추가.
- 실제 프론트 + 가짜 Tauri 경계 **55/55 통과**: 네 모드의 버튼 노출·겹침·경계·클릭 영역·명령 호출, 활성/비활성 카드 버튼 열, 삭제 확인 시 위치 유지 포함. 프론트 단위 검사 93개와 기존 배치 검사 8개도 통과.
- Rust 로직은 이번 후속에서 변경하지 않았다. 네이티브 WKWebView·Windows 실행 검증의 기존 한계는 그대로다.

[네 모드 비교 이미지](design/four-modes.png) · [원본 크기 비교 HTML](design/four-modes.html) · [55개 검사 결과](four-mode-test-results.json)

재현: `npm run build && node scripts/test-workspace-ui.mjs --all-modes docs/design`
