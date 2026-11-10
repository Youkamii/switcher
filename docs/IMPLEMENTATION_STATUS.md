# 개선 적용 현황

작업 브랜치: `feat/connection-workspace`

| 작업 | 이슈 | 상태 |
| --- | --- | --- |
| UI 재설계와 레이아웃 안정화 | [#179](https://github.com/Youkamii/switcher/issues/179) | 디자인 시안 준비, 구현 진행 |
| 인증 오류 분류와 재연결 | [#180](https://github.com/Youkamii/switcher/issues/180) | 구현 예정 |
| Claude/Codex API 키 프로필 | [#181](https://github.com/Youkamii/switcher/issues/181) | 구현 예정 |
| 업데이트 서명 검증 | [#182](https://github.com/Youkamii/switcher/issues/182) | 구현 예정, 배포 신뢰키 설정 필요 |

[개선 계획](IMPROVEMENT_PLAN.md) · [수정 전 검증 결과](REVIEW_TEST_RESULTS.md) · [디자인 시안 HTML](design/index.html) · [디자인 시안 이미지](design/preview.png)

디자인 시안은 가짜 데이터만 사용하며 실제 계정이나 API 키를 변경하지 않는다. 저장소 루트에서 `python3 -m http.server 1430 --bind 127.0.0.1 --directory docs/design`으로 열고 http://127.0.0.1:1430 에서 확인할 수 있다. 실제 키 입력은 비활성화했다.
