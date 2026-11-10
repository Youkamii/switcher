# 개선 적용 현황

작업 브랜치: `feat/connection-workspace`

| 작업 | 이슈 | 상태 |
| --- | --- | --- |
| UI 재설계와 레이아웃 안정화 | [#179](https://github.com/Youkamii/switcher/issues/179) | 시안 준비. 빈자리·카드 이동 수정 및 레이아웃 8/8 통과. 화면 재구성·네이티브 검증 진행 예정 |
| 인증 오류 분류와 재연결 | [#180](https://github.com/Youkamii/switcher/issues/180) | 구현 예정 |
| Claude/Codex API 키 프로필 | [#181](https://github.com/Youkamii/switcher/issues/181) | 구현 예정 |
| 업데이트 서명 검증 | [#182](https://github.com/Youkamii/switcher/issues/182) | 구현 예정, 배포 신뢰키 설정 필요 |

[개선 계획](IMPROVEMENT_PLAN.md) · [수정 전 검증 결과](REVIEW_TEST_RESULTS.md) · [디자인 시안 HTML](design/index.html) · [디자인 시안 이미지](design/preview.png)

디자인 시안은 가짜 데이터만 사용하며 실제 계정이나 API 키를 변경하지 않는다. 저장소 루트에서 `python3 -m http.server 1430 --bind 127.0.0.1 --directory docs/design`으로 열고 http://127.0.0.1:1430 에서 확인할 수 있다. 실제 키 입력은 비활성화했다.

## 적용 검증

- UI 결함 수정: 프론트 93개 통과, 실제 CSS 배치 검사 8개 통과, TypeScript/Vite 빌드 성공.
- 기존 호버 전용 UI를 강제하던 검사는 새 고정 액션 요구에 맞게 교체하고, 손잡이 크기의 조합 검사를 추가했다.
- 실행 중인 실계정 앱은 아직 교체하지 않았다.
