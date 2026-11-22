# 개선 적용 현황

작업 브랜치는 `feat/connection-workspace`다. 2026-11-20 로컬 빌드에 계정별 API 키와 Claude 조직 잔액을 반영했다. 2026-11-22에는 그 위의 `feat/api-token-click`에서 보기 모드를 셋으로 줄이고 위젯 모드의 구독 ↔ API 전환을 넣었다(바로 아래 절). 공개 앱·npm 패키지는 2026-11-17에 게시한 `v2.1.2`이며, 이 버전에는 Codex 공유 서버 자동 갱신이 포함돼 있다.

[전체 화면 한 장](design/complete-preview.png) · [원본 크기 보기](design/complete-preview.html) · [API 등록·사용법](API_CONNECTIONS.md)

## 보기 모드 3종 · 위젯 모드의 구독 ↔ API 전환 · 2026-11-22 로컬 빌드

브랜치 `feat/api-token-click`(`feat/connection-workspace` 위)의 로컬 빌드다. 푸시·배포 전이다. 이 절 아래의 "네 모드"·"컴팩트"는 각 날짜 당시의 기록이다.

| 이슈 | 바뀐 것 | 커밋 |
| --- | --- | --- |
| [#183](https://github.com/Youkamii/switcher/issues/183) | 2 컴팩트를 없앴다. 보기 모드는 **1 일반 / 2 미니멀 / 3 엣지**. 저장돼 있던 컴팩트 선택은 미니멀로 옮긴다. | `acfb499` |
| [#184](https://github.com/Youkamii/switcher/issues/184) | 엣지의 펼친 판에서 누를 수 있는 카드에 호버를 그린다. macOS의 비활성 패널에는 `:hover`가 오지 않아 러스트 커서 폴링으로 그린다. | `5bc7886` |
| [#185](https://github.com/Youkamii/switcher/issues/185) | 엣지 손잡이의 높이. 패널이 길어 창을 더 옮길 수 없을 때 ☰를 화면 위·아래 끝 너머로 끌면 손잡이가 그만큼 옮겨진다. | `5bc7886` |
| [#186](https://github.com/Youkamii/switcher/issues/186) | 미니멀·엣지에서 사용 중인 계정을 한 번 더 누르면 그 계정의 API 키, 다시 누르면 구독. API 상태는 테마별 API 색으로 표시한다. [사용법](API_CONNECTIONS.md#위젯-모드에서-구독--api) | `73e899e` |
| [#188](https://github.com/Youkamii/switcher/issues/188) | 엣지의 접힌(작은) 손잡이를 누른 채 위아래로 끌어 벽 끝에서 끝까지 옮긴다. 높이·벽·판 안에서의 손잡이 자리를 저장해 위젯을 다시 띄워도 그대로이고, 판 높이가 바뀌어도 손잡이는 화면에서 제자리다. #185의 "☰를 끝 너머로 끌기"는 판을 옮기는 길로 남는다. | `68fd83a` |
| [#187](https://github.com/Youkamii/switcher/issues/187) | 일반 모드 제목 줄의 **API 키 추가**는 계정을 고르지 않는다. 키만 넣으면 주인 계정을 찾아 그 계정 밑에 저장하고, 못 찾을 때만 묻는다. [규칙](API_CONNECTIONS.md#계정을-고르지-않고-추가하기) · [화면](design/api-key-only.png) | `daa5036`, `51dc73a` |

### 검증

Rust 262개(무시 23개 제외), 프론트 단위 131개, Chromium 화면 검사 300개, macOS WKWebView 화면 검사 247개, 배치 검사 7개가 통과했다. 화면 검사는 빌드된 프론트와 가상 계정을 쓴다. [Chromium 검사 원문](view-mode-test-results.json) · [WKWebView 검사 원문](view-mode-webkit-test-results.json) · [세 모드와 `API ›` 예고 화면](design/view-modes.png)

```bash
npm test
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
node scripts/test-workspace-ui.mjs --all-modes docs/design --output docs/view-mode-test-results.json
node scripts/test-workspace-ui.mjs --webkit --output docs/view-mode-webkit-test-results.json
node scripts/review-layout.mjs
```

실제 앱의 포인터 동작은 macOS 27.2에서 격리해 확인했다. 가짜 홈(`HOME`·`CFFIXED_USER_HOME`)과 그 안에 새로 만든 키체인, 합성 계정·합성 키, 네트워크 차단(`sandbox-exec`) 상태로 번들 실행 파일을 띄우고 실제 마우스 이벤트를 보냈다. 실계정·실키·실제 CLI 설정은 쓰지 않았고, 실행 전후 실제 설정 파일의 해시가 같았다.

| 확인한 동작 | 결과 |
| --- | --- |
| 엣지: 사용 중 카드의 `API ›`를 누르고, 같은 자리를 다시 누름 | API 키 적용 → 구독 복귀. 전환 직후 포인터 밑은 API 칸이며, 틀 전체가 복귀 대상이다 |
| 엣지: 계정 이름 줄을 누름 | 구독 복귀 |
| 엣지: 0.76초 박자로 6번 누름 | API·구독·API·구독·API·구독 — 매번 전환 |
| 엣지: 빠른 더블클릭 (사용 중 카드 / API 틀) | 각각 한 번만 전환 (API 적용 / 구독 복귀) |
| 엣지: 다른 계정을 누름 | 그 계정의 구독으로 전환 |
| 미니멀: 더블클릭으로 다른 계정 → 다시 더블클릭 → 같은 자리 더블클릭 | 계정 전환 → API 키 적용 → 구독 복귀 |
| 미니멀: 1.1초 박자로 더블클릭 4번 | API·구독·API·구독 — 매번 전환 |
| 미니멀: Codex 계정에서 더블클릭 4번 | `auth.json`·`config.toml`이 API 키 ↔ ChatGPT 로그인을 두 번 왕복 |
| 일반: Claude 제목 줄 **API 키 추가** → 합성 키 입력 → 저장 (계정 둘, 네트워크 차단) | 아무것도 저장하지 않고 계정을 물었다. 입력한 키는 창에 남았다 |
| 일반: 위 창에서 둘째 계정을 고르고 저장 | 그 계정 밑에 `api`로 저장. 키는 가짜 키체인에만 있고 메타데이터에는 없다 |
| 일반: Codex 제목 줄 **API 키 추가** → 합성 키 입력 → 저장 (계정 하나) | 묻지 않고 그 계정에 저장. 구독 인증은 그대로 |
| 엣지: 저장된 자리 없이 실행 (#188) | 예전과 같다 — 화면 오른쪽 아래, 손잡이는 창 세로 중앙 |
| 엣지: 왼쪽 벽·벽의 30% 높이·판 안 100px를 저장해 두고 다시 실행 | 왼쪽 벽에 붙고(x 0) 창과 손잡이가 저장한 자리에 선다. 저장값은 바뀌지 않는다 |
| 엣지: 벽 맨 아래 / 맨 위를 저장해 두고 다시 실행 (오른쪽·왼쪽 벽) | 창이 작업영역 바닥(1153) / 꼭대기(38)에 닿고 손잡이가 벽 끝에 선다 |
| 실제 위젯: 보이던 자리(창 269, 손잡이 화면 858)를 저장하고 새 빌드로 다시 실행 | 같은 자리(1840, 269)에 그대로 뜬다 — 전에는 다시 띄울 때마다 화면 아래로 돌아갔다 |

재시작 복원은 화면이 잠긴 상태에서 창 좌표와 러스트 진단 로그로 확인했다(입력이 필요 없다). **손잡이를 실제 포인터로 끄는 확인은 하지 못했다** — 화면이 잠겨 있어 합성 입력을 보낼 수 없었다. 끌기의 판정·창 따라오기·저장·클릭 삼킴은 Chromium과 WKWebView 화면 검사가 포인터 이벤트로 본다(창 이동이 늦게 반영되는 가짜 창 사용).

실계정에서 관측한 것(2026-11-22, Claude Code 2.1.328): 구독으로 작업 중이던 세션이 API 선택 직후의 요청에서 `billing_error`("Credit balance is too low")로 거절됐고, 구독으로 돌아오자 이어졌다. 그 키의 조직은 이틀 전 Console에 크레딧이 표시된 곳이었다 — 플랜의 월간 API 크레딧은 대화형 Claude Code에 쓰이지 않는다는 [공식 안내](https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans)와 맞는다. 같은 키를 환경변수로 준 `claude -p`는 사용자 승인하에 실제로 호출해 Haiku 5.5·Fable 5.2 모두 응답을 받았다(합계 약 0.11달러). 월간 크레딧은 이 경로로 쓴다 — [쓰는 법](API_CONNECTIONS.md). Claude Code의 API 선택·해제는 실행 중인 세션에도 바로 적용된다는 뜻이어서 앱과 문서의 "다음 CLI 실행부터" 안내를 고쳤다.

조직 이름으로 주인을 찾는 경로는 격리 앱에서는 볼 수 없다(네트워크를 막는다). 대신 로컬 전용 검사 `real_stored_keys_are_matched_to_the_accounts_they_are_linked_to`로, 이 컴퓨터에 저장된 실제 키가 저장된 세 계정 가운데 지금 연결된 계정으로 짚이는 것을 확인했다(읽기만 하고 과금·저장이 없다). 가짜 서버를 쓴 통합 검사는 조직 이름 대조·주인 불명·무조회 경로를 본다.

남은 범위:

- 엣지 손잡이 끌기(#188)를 실제 포인터로 확인하지 못했다(macOS: 화면 잠금, Windows: 실기기 없음). 볼 것: 접힌 손잡이를 눌러 끌 때 판이 접히고 손잡이가 따라오는지, 창 밖으로 커서가 나가도 끊기지 않는지, 놓은 뒤 카드가 눌리지 않는지.
- Windows 실기기의 포인터 동작은 확인하지 못했다 — 엣지의 클릭 전환과 색, 미니멀의 더블클릭 전환, ☰로 판을 옮긴 뒤 손잡이 자리, `API` 칩의 글자 폭, 제목 줄 **API 키 추가** 창과 계정 목록의 모양(WebView2).
- 실제 키를 새로 등록해 조직 이름으로 다른 계정에 들어가는 것은 사용자의 키가 하나뿐이라 해 보지 못했다(이미 등록된 키의 주인 판별만 확인).
- `docs/screenshot.png`(README 첫 그림)는 아직 네 모드 화면이다.
- 실제 유료 API 호출과 실키 적용은 수행하지 않았다.

## 계정별 API 키 · 2026-11-20 로컬 빌드

일반 모드의 **계정 추가**는 로그인을 시작한다. 카드를 만든 뒤 **API 키 추가**로 키를 저장한다. 한 계정에 여러 키를 추가할 수 있고, 서로 다른 계정에서 같은 키 이름을 쓸 수 있다. **저장하고 사용**과 키 옆의 **사용**이 CLI 인증을 전환한다. 컴팩트·미니멀·엣지는 사용 중인 키와 잔액을 소속 계정 아래에 표시한다. [현재의 세 모드 화면](design/view-modes.png)

기존 키 하나를 사용자가 지정한 실계정에 연결하고, 키 버전·현재 CLI 선택·Console 잔액 파일이 보존됐음을 확인했다. 계정 이름 변경, 삭제 후 재연결, 동일한 키 이름의 독립 전환, 소속 계정으로 범위를 좁힌 잔액 조회는 가상 계정으로 검사했다.

### 요구사항별 재검증

macOS 설치본과 로컬 빌드의 실행 파일 해시가 일치했고 앱 서명 검증을 통과했다. Rust 249개, 프론트 단위 106개, Chromium 183개, macOS WKWebView 147개, 배치 검사 8개가 통과했다. 화면 검사는 실제 배포용 프론트와 가상 계정을 사용했다. [Chromium 검사 결과](four-mode-test-results.json) · [WKWebView 검사 결과](webkit-test-results.json)

| 요구사항 | 확인한 동작 |
| --- | --- |
| 계정 하위 API 키와 버튼 배치 | Claude 두 번째 카드에 키를 등록하고 다른 계정에도 같은 이름의 키를 저장했다. **사용**은 클릭한 카드의 키를 선택했다. 네 모드에서 선택한 키가 원래 계정 아래에 표시됐고 도구 버튼 5개가 겹치지 않았다. 일반 모드에서는 호버와 삭제 확인 중에도 다음 카드와 버튼 열이 제자리를 유지했다. |
| 미등록 계정의 빈 공간 | 등록 계정과 키가 모두 0개일 때 AI 표시를 켠 상태와 끈 상태를 비교했다. 컴팩트·미니멀·엣지에서 창 높이는 각각 263·157·317px, SYSTEM 시작 위치는 각각 93·38·36px로 두 엔진 모두 차이가 0px였다. 일반 모드에는 공급자별 **계정 추가**와 미등록 안내가 보였다. |
| 키 저장과 CLI 전환 | 격리된 macOS 키체인에서 합성 키 저장·읽기·정리를 확인했다. 설치한 Claude Code 2.1.328과 Codex 0.169.0이 선택한 API 인증을 별도 실행 명령 없이 인식했다. 이전 구독 인증과 사용자 설정 복원은 Rust 검사로 확인했다. |
| 토큰 갱신과 실행 중 Codex | 비활성 계정 갱신, 활성 토큰 보호, 갱신 거부 시 재로그인 안내를 검사했다. 설치한 Codex의 관리되는 공유 서버는 진행 중인 작업이 끝난 뒤 두 번째 테스트 계정으로 전환했고, 같은 대화가 복원됐다. 단독 프로세스는 다음 실행에서 선택한 계정을 읽었다. |
| Claude 잔액 | 실제 등록 키로 조회했을 때 해당 조직의 Console 로그인 필요 상태가 반환됐다. 이 상태에서는 앱에 마지막으로 확인한 조직 크레딧과 확인 시각이 표시된다. 자동 조회 성공·실패와 통화 표시, 타 계정 데이터 혼입 방지는 테스트 응답으로 검사했다. OpenAI 잔액은 결제 화면에서 확인한다. |
| 등록 중 창 이동 | 두 화면 엔진에서 등록·교체·완료 창 제목이 네이티브 드래그를 호출하고 입력값이 보존됐다. 화면 제어 도구가 설치본의 macOS 패널을 선택하는 단계에서 실패해, 마우스 드래그 전후의 실제 창 좌표는 검증하지 못했다. |
| Windows | 공개 v2.1.2의 네이티브 검증 기록은 아래에 있다. 이번 계정별 키 변경의 Windows 네이티브 실행 결과는 아직 없다. |

계정 갱신용 로컬 테스트 서버의 소켓 읽기가 간헐적으로 실패해, 연결을 수락한 뒤 블로킹 모드로 전환하도록 고쳤다. 해당 검사를 다섯 번 연속 통과했다.

## API 크레딧 잔액 · 2026-11-20 로컬 빌드

월 예산·사용률 막대·관리자 키 입력을 제거했다. 네 가지 위젯 모드가 Claude의 조직 선불 잔액 또는 Console 대시보드에서 가져온 조직 크레딧과 확인 시각을 표시한다. 개인 계정에도 적용된다. 실제 등록 키로 조직을 확인하고 사용자가 제공한 대시보드와 대조했다. 현재 구독 토큰의 Console 조회는 권한 거부였으며, 대시보드 확인값을 로컬에 저장한 뒤 다시 읽어 금액·통화·시각을 확인했다. OpenAI는 공식 결제 화면에서 확인한다. [사용법](API_CONNECTIONS.md) · [현재의 세 모드 예시](design/view-modes.png)

계정별 키를 도입하기 전 잔액 빌드에서 macOS Rust 246개, 프론트 단위 106개, Chromium 153개, macOS WebKit 118개와 배치 검사 8개가 통과했다. 당시 macOS 앱 번들과 ad-hoc 서명도 검증했다.

## Codex 공유 서버 자동 갱신 — v2.1.2

- 구독 전환의 기존 백업 → 파일 교체 순서를 유지하고, 그 뒤 Codex 0.167.0 이상 정식 버전의 관리되는 공유 서버 갱신을 예약한다. 일반 버튼·고정 모드·TFSD가 같은 코어를 사용한다. 공유 서버가 없으면 새 서버를 임의로 시작하지 않는다.
- 진행 중인 작업·승인 대기·저장되지 않는 대화는 재시작하지 않는다. idle 확인 뒤 정상 종료만 요청하고, 이전 프로세스가 종료된 뒤 공식 daemon 명령으로 대체 서버를 시작한다. 강제 종료와 외부 토큰 공급 방식은 사용하지 않는다. 별도 helper가 위젯 종료 뒤에도 마무리한다.
- 선택된 계정과 실행 중 연결의 반영 상태를 분리해 대기·갱신 중·완료·실패를 표시한다. 실패한 갱신은 기존 새로고침 버튼으로 재시도한다. 단독 실행·구형 CLI·Windows 관리자 실행은 `codex resume` 안내를 유지한다. API 활성화의 실행 중 세션 전환은 이번 범위에 포함하지 않는다.
- 로컬 Codex 0.167.0·0.169.0, 가짜 OAuth 두 계정과 로컬 HTTP 응답으로 **진행 중 작업을 기다림 → B 인증 반영 → 기존 thread ID와 대화 내용 복원 → managed OAuth 유지**를 검사했다. 추가로 격리 PTY의 실제 Codex 0.169.0 TUI와 로컬 HTTPS 서버를 사용해 **터미널 프로세스 재실행 없이 자동 재연결 → 같은 대화에서 후속 입력 → 이전 답변이 후속 요청의 문맥에 포함**되는 것까지 통과했다. 실제 계정·모델 API는 사용하지 않았다. 일반 `-c` 옵션을 주입하는 로컬 래퍼는 단독 실행으로 전환되므로 이 자동 갱신 범위에서 제외됨도 확인했다.
- 로컬 검증: Rust 전체 243 통과·19 ignored, 빌드된 macOS 앱의 headless helper를 사용한 설치 Codex 통합 검사 통과, TypeScript/Vite 및 프론트 단위 104 통과, Chromium 네 모드 139·WebKit 106·배치 8 통과.
- [Windows CI](https://github.com/Youkamii/switcher/actions/runs/44033190107)는 `8bfbce0`에서 성공했다. 프론트 단위 104, Chromium UI 76, Rust 291(17 ignored), 설치 CLI 인증·기존 세션 캐시 검사, 포터블 EXE 빌드·npm 버전 인식이 통과했다. 별도 일반 사용자로 **빌드된 EXE의 headless helper + Codex 0.167.0**을 실행해 작업 중 대기, 새 계정 반영, 같은 thread ID·대화 복원, managed OAuth 유지도 통과했다. Windows 소켓의 전체 경로를 일반 파일처럼 여는 확인을 상위 폴더 기준 확인으로 수정하고 실제 AF_UNIX 소켓 회귀 검사를 추가했다. Windows TUI 화면의 실제 조작은 이 검사 범위에 포함되지 않는다.

## v2.1.2 배포 (2026-11-17)

- 태그 `v2.1.2` / 코드 `4c815b3`로 [Windows·macOS 릴리스](https://github.com/Youkamii/switcher/actions/runs/44034219218)와 [npm 게시](https://github.com/Youkamii/switcher/actions/runs/44035204221)가 성공했다. 같은 배포 커밋의 [Windows 전체 검사](https://github.com/Youkamii/switcher/actions/runs/44034208729)도 성공했다.
- 공개 레지스트리에서 `switcher-widget@2.1.2`, `latest=2.1.2`, 태그와 같은 `gitHead` 및 provenance 존재를 확인했다. 새 임시 폴더에 공개 npm 패키지를 설치하고 실제 `ensureDist()`로 앱을 받았으며, 두 번째 호출은 네트워크 없이 캐시를 인식했다.
- macOS 다운로드 앱의 버전 `2.1.2`, arm64 형식, ad-hoc 서명 무결성을 확인했다. **공개된 macOS 앱의 headless helper**로 가짜 계정·로컬 모델 통합 검사도 다시 통과했다. 사용자 계정이나 실행 중인 실제 Codex 세션은 변경하지 않았다.
- Windows ZIP의 GitHub SHA-256·크기·x64 PE·버전 표식과 EXE 리소스 버전 `2.1.2.0`을 확인했다. 업데이트 manifest는 두 운영체제 모두 `v2.1.2` 자산을 가리킨다. 구버전에서 넘어올 때 필요한 고정 이름 자산은 ID·크기·SHA-256이 배포 전과 같음을 확인했다.
- 기존 앱은 **위젯 우클릭 → 업데이트 확인**으로 갱신한다. Codex 0.167.0 이상 정식 버전의 관리되는 공유 서버를 사용하는 구독 세션이 자동 갱신 대상이다. 단독 실행·설정 주입 래퍼·Windows 관리자 실행은 위의 전환 범위를 따른다.

## Windows API 진입·Codex 세션 후속 수정 (2026-11-17)

- v2.1.0 코드의 Claude/Codex 추가 진입은 구독/API 선택 창이다. 사용자가 보고한 Windows의 직접 로그인 진입 원인은 실행본 버전 확인 전이므로 확정하지 않았다. 이미 실행 중인 이전 앱이 단일 인스턴스로 재표시되는 경우도 구분해야 한다.
- npm 실행기가 `bin-dist`에 파일만 있으면 버전을 확인하지 않던 결함을 수정했다. Windows EXE 버전 리소스·macOS 번들 버전을 확인하고 오래되거나 손상된 캐시만 정확한 npm 버전으로 교체한다. 새 다운로드는 임시 위치에서 버전을 확인하며, 실패 시 기존 앱을 보존하고 자동 갱신된 더 최신 앱을 되돌리지 않는다.
- 단일 추가 버튼을 **계정·API 추가**, 선택 항목을 **API 키 등록**으로 명확히 표시했다. 추가 버튼 클릭만으로 구독 로그인을 시작하지 않는 것을 배포 프론트로 검사한다.
- Codex CLI 0.167.0의 실행 중 인증 캐시를 실제 설치 CLI와 가짜 OAuth 두 계정으로 재현했다. 파일을 B로 교체해도 기존 프로세스는 A, 새 프로세스는 B를 읽는다. **v2.1.1에는 실행 중인 세션의 즉시 전환이 포함되지 않았다.** 해당 버전의 일반 화면은 **선택됨**과 다음 실행 안내, 작은 모드는 계정 툴팁으로 범위를 표시한다. 공유 서버 갱신 후속 구현은 위 항목에 기록했다.
- 로컬 검증: 단위 검사 104 통과, Rust 239 통과·18 ignored, Chromium 화면 134 통과, WebKit 화면 101 통과, 배치 8 통과, TypeScript/Vite 및 macOS 앱 빌드 성공. `v2.1.1` 앱의 실제 번들 버전과 npm 실행기의 인식, ad-hoc 서명 무결성도 확인했다.
- [Windows CI](https://github.com/Youkamii/switcher/actions/runs/44005695608)는 수정 코드 `eb2552d`에서 성공했다. 단위 104개, Windows Chromium의 배포 프론트 71개, Rust 286개(16 ignored), 설치 CLI 인증 검사, Codex 기존/신규 프로세스 인증 구분, 포터블 EXE 빌드와 npm 실행기의 네이티브 버전 인식을 모두 통과했다. Windows Chromium 검사는 Windows WebView2 실제 포인터 조작을 대신하지 않는다.

## v2.1.1 배포 (2026-11-17)

- 태그 `v2.1.1` / 코드 `eab88df`로 [Windows·macOS 릴리스 빌드](https://github.com/Youkamii/switcher/actions/runs/44006616119)와 [npm 게시](https://github.com/Youkamii/switcher/actions/runs/44007715659)가 성공했다.
- npm 공개 처리 대기 후 `switcher-widget@2.1.1`, `latest=2.1.1`, `gitHead=eab88df` 일치와 provenance 존재를 확인했다. 새 임시 폴더에 실제 공개 패키지를 설치하고 배포용 `ensureDist()`로 macOS 앱을 받았다. 두 번째 호출은 네트워크 없이 현재 버전 캐시를 인식했다. 앱을 실행하거나 사용자 인증을 바꾸지 않았다.
- 다운로드된 macOS 번들 버전 `2.1.1`과 ad-hoc 서명 무결성을 확인했다. Windows ZIP은 GitHub SHA-256·파일 크기 일치, x64 PE, ZIP 버전 표식과 EXE 버전 리소스 `2.1.1.0`을 확인했다. 업데이트 manifest의 두 운영체제 주소도 모두 `v2.1.1`을 가리킨다.
- 실행 중인 구버전 앱은 **위젯 우클릭 → 업데이트 확인**으로 갱신한다. Codex 실행 중 세션의 즉시 전환은 이 패치에 포함되지 않으며, 위의 다음 실행 적용 범위를 따른다.

## v2.1.0 배포

- 태그 `v2.1.0` / 코드 `5ee97fc`로 [Windows·macOS 릴리스 빌드](https://github.com/Youkamii/switcher/actions/runs/43658372184) 성공. Windows Rust 286개, macOS Rust 239개, 로컬 프론트 93개 검사 통과.
- [npm 게시](https://github.com/Youkamii/switcher/actions/runs/43659035174) 성공. 공개 레지스트리의 `switcher-widget@2.1.0`, `latest=2.1.0`, `gitHead=5ee97fc` 일치와 provenance 존재를 확인했다. npm 서버의 게시 처리 대기 후 실제 조회·설치까지 확인했다.
- 새 임시 폴더에서 npm 패키지를 설치하고 실제 배포용 `ensureDist()`로 macOS 앱을 다운로드했다. 번들 버전 `2.1.0`, arm64 실행 파일, ad-hoc 코드 서명 무결성을 확인했다. 이 설치 검사는 앱을 실행하거나 사용자 계정을 변경하지 않았다.
- 게시된 Windows ZIP의 버전 표식 `2.1.0`, x64 실행 파일, 파일 크기와 GitHub SHA-256 일치를 확인했다. 앱 업데이트 manifest도 두 운영체제의 `v2.1.0` 파일을 가리킨다. 기존 `v1.8.5` 다운로드 보관함과 구버전 자산은 유지했다.

## 적용한 동작

| 요청 | 적용 내용 |
| --- | --- |
| 버튼 모양·나열 | 계정 카드 우측에 전환/사용 중 슬롯과 SVG 아이콘 3개(다시 연결·이름·삭제)를 배치. 활성·비활성 계정의 위치가 같다. 호버·삭제 확인 때 다음 카드가 움직이지 않는다. |
| 다른 도구가 안 보임 | 메모·자동 전환·가리기·잠자기 방지·화면 끄기를 기본 펼침으로 표시. 같은 선 두께의 아이콘을 사용하며 일반 모드에는 이름도 표시. |
| 네 모드 유지 | 일반 420px·컴팩트 280px·미니멀 120px·엣지 80px 유지. 일반 화면에서 네 모드를 직접 선택할 수 있다. |
| AI 미사용 빈자리 | 숨긴 제공자 행·구분선의 공간 제거. 엣지 손잡이 높이도 실제 표시 행에 맞춰 계산. AI를 모두 숨기면 SYSTEM과 도구만 남는다. |
| 잦은 인증 오류 | CLI를 실행한 상태에서도 다른 계정의 사용량·토큰 갱신이 진행된다. 활성 계정의 토큰은 CLI가 관리하고 위젯은 중복 회전하지 않는다. 액세스 토큰 갱신 필요, 갱신 토큰 거부, 권한 오류, 요청 제한을 구분. 마지막 사용량을 보여줄 때도 실패 이유를 남긴다. 카드의 다시 연결에서 기존 계정을 재인증할 수 있다. |
| API 등록과 사용 | 계정 카드의 **API 키 추가**에서 저장하거나 즉시 사용한다. 같은 카드에서 기존 키의 사용·해제·확인·교체·삭제를 관리한다. 적용 실패 시 저장한 키로 재시도할 수 있다. |
| API 크레딧 | 키 발급 조직 확인, Console 로그인 기반 선불 잔액 조회, 출처·시각이 있는 대시보드 확인값, 결제 화면 바로가기. |
| API 키 보호 | 저장 연결은 macOS Keychain/Windows DPAPI. Claude는 보안 저장소 helper, Codex는 표준 활성 auth.json(Unix 0600)을 사용한다. OAuth·설정을 먼저 백업하고 해제·구독 전환·삭제 시 복원한다. 키 입력·목록·로그 보호 유지. |
| 상태·오류 복구 | 확인 상태와 시각을 저장. 키 교체 시 초기화, 확인 중 교체·삭제 시 낡은 결과 폐기. 동일 이름의 암묵적 교체 차단. 보안 저장 실패 시 메타데이터 복구. |
| 등록 중 창 이동 | 등록·교체·완료 화면의 제목과 원래 타이틀바 영역에서 네이티브 창 드래그. 입력값 유지, 입력칸·버튼·우클릭 제외. |
| 비활성 macOS 창 | API 창 닫기와 키 입력 정리를 DOM close 이벤트·타이머에만 맡기지 않고 즉시 처리. 삭제 확인은 실제 경과 시간과 사용자 동작으로 해제. |

구독 토큰 수명을 바꾸거나 만료 자체를 없앤 것은 아니다. 활성 파일 먼저 백업 → 대상 프로필 복사 순서, 활성 계정 직접 갱신 보호, 프로필별 갱신·전환 잠금과 사이드카 복구는 유지했다. **Claude/Codex를 실행 중이라는 이유로 다른 계정의 갱신을 막지 않는다.** `f28add7`에서 도입했던 제공자 전체 차단·종료 감시·“CLI 사용 중” 안내는 잘못된 제약이라 제거했다. API는 앱에서 명시적으로 선택하며 다음 일반 CLI 실행부터 적용된다. API 선택 시 TFSD를 해제하고, 구독으로 돌아가면 앱이 적용한 설정만 복원한다. 자동 과금 대체는 없다.

## v2.1.0~v2.1.2 검증 기록

2026-11-13 푸시 전 재검증: Rust 239, API 집중 24, 프론트 단위 93, Chromium 123, WebKit 93, 배치 8 통과. TypeScript/Vite와 macOS 앱 빌드도 성공했다. Chromium 첫 실행은 디버깅 포트 생성 대기 시간 초과로 중단됐고, 단독 재시도에서 전체 검사가 통과했다. 새 인증 복원 보관소 `api-active/`를 Git 제외 규칙에 추가하고 실제 제외 여부를 확인했다. 푸시 대상 기존 16개 커밋의 138개 새 blob에서 인증 파일명과 알려진 토큰 패턴을 검사했으며 발견 사항은 없었다. 추가 Windows 실행 검증은 아래 표에 반영했다.

| 검사 | 결과 |
| --- | --- |
| 프론트 단위 검사 `npm test` | 93 통과 |
| Rust `cargo test --lib` | 239 통과 / 18 ignored |
| CLI 사용 중 갱신 회귀 검사 | Claude/Codex 이름의 무해한 프로세스를 실행한 상태에서 다른 계정은 1회 갱신 성공·재사용, 활성 계정은 요청 0건·토큰 무변경. 두 제공자 모두 로컬 서버로 검사 |
| API 집중 검사 `cargo test api_connections::tests -- --include-ignored` | 11 통과. 실제 macOS 키체인 가짜 키 왕복·삭제, 설치된 Claude/Codex `--version` 실행 포함 |
| API 선택 집중 검사 `cargo test api_connections::activation::tests -- --include-ignored` | 7 통과. 설정·OAuth 백업/복원, 외부 변경 보존, 쓰기 실패 복구. 설치된 Claude/Codex의 실제 인증 상태 명령으로 API 인식 확인(가짜 키·격리 홈), 활성 연결 삭제 복원 |
| API 비용 집중 검사 | 6 통과. 연결별 $195 기본값, UTC 월 변경, 0/미조회 구분, 초과/잘못된 금액, 관리자 키 분리·저장 실패 복구, HTTP 단위·페이지 합산·ID 필터·실패·응답 경합 |
| Chromium 실제 번들 검사 | 123 통과. 통합 계정 추가·네 모드·AI 숨김·API 전환/복원·사용량 입력/자동조회/초과/오류·드래그 호출·입력 보존 검사 |
| macOS WebKit 실제 번들 검사 | 93 통과. 비활성 NSPanel에 WKWebView를 붙여 같은 기능·배치 검사 수행 |
| 기존 숨김·배치 회귀 검사 | 8 통과. 최초 기준선의 4건 실패 해소 |
| 등록 계정 0개 화면 (2026-11-15) | Chromium 45, WebKit 45 통과. 컴팩트·미니멀·엣지에서 AI 섹션 제거, AI 숨김 상태와 동일한 본문 위치·창 높이, 엣지 AI 막대·구분선 제거 확인. 일반 관리 화면은 계정 추가 안내 유지 |
| TypeScript/Vite | 빌드 성공 |
| macOS Tauri 앱 | 빌드 성공. ad-hoc 서명 검증 성공, notarization 없음 |
| Windows 프론트 | 단위 검사 93개와 TypeScript/Vite 빌드 통과. CRLF 소스에서도 검사 가능하도록 두 검사 파일 수정 |
| Windows Rust·API 통합 | 전체 Rust 검사 통과. 실제 DPAPI 키 저장·복호화·교체·손상 거부·삭제, PowerShell 키 반환, API 해제·구독 전환·활성 연결 삭제 시 OAuth 복원 확인 |
| Windows 설치된 CLI | Claude Code 2.1.300 / Codex 0.167.0의 인증 상태 명령으로 API 인식 확인. 가짜 키·격리 홈 사용 |
| Windows 포터블 앱 | `npm run tauri build -- --no-bundle` 성공 |

[Windows CI 실행 기록](https://github.com/Youkamii/switcher/actions/runs/43406000742) — 실행 코드 `1c6dea1`. 최초 프론트 검사 9건은 CRLF 소스에서 함수 구간을 찾지 못해 실패했고 줄바꿈 정규화로 수정했다. 다음 실행에서는 PowerShell helper가 올바른 키를 반환하고도 stderr가 비어 있어야 한다는 새 테스트 조건 때문에 1건 실패했다. 종료 코드·정확한 stdout·stderr의 키 노출 여부로 계약을 바로잡은 뒤 전체 검사와 빌드가 통과했다. 이 과정에서 앱의 API 구현 코드는 변경하지 않았다.

[Chromium 검사 원문](four-mode-test-results.json) · [WebKit 검사 원문](webkit-test-results.json) · [최초 리뷰·기준선](REVIEW_TEST_RESULTS.md)

UI 검사는 **실제 배포 프론트 + 가짜 Tauri 응답**을 사용한다. WebKit은 임시·비영속 데이터 저장소와 화면 밖 NSPanel에서 실행하며, 비활성 페이지의 타이머 지연 때문에 테스트 동작 간 진행은 네이티브 테스트 호스트가 구동한다. 실제 사용자 프로필·키·기존 브라우저 세션을 사용하지 않는다. 이미지의 계정·사용량·연결 확인 결과도 검증용 예시다. 운영 앱은 새 빌드 실행 파일 경로와 새 프로세스를 확인했다. 최종 실행은 사용자가 선택한 엣지 모드(80px)이며, 앞선 실행에서 일반 모드(420px)도 확인했다. 운영 창의 접근성 트리와 캡처가 정상 반환되지 않아 네이티브 포인터로 API 등록을 끝냈다고 기록하지 않는다. 버튼 동작은 별도 WebKit 검증 호스트에서 검사했다. 등록창의 드래그는 Tauri 네이티브 호출 전달과 이동 이벤트 이후 입력 보존까지 확인했으며, 운영 창을 실제 포인터로 끌어 이동한 검증과 구분한다. 사용자 요청으로 기존 저장 API를 실제 적용한 뒤 Claude 인증 상태 명령에서 loggedIn=true, authMethod=api_key_helper, apiKeySource=apiKeyHelper를 확인했다. 키 원문·계정 정보는 출력하지 않았다.

API HTTP 검사는 로컬 서버에서 200(정상/잘못된 본문)·401·403·429·503·리다이렉트, 인증 헤더, 안전한 오류 표시를 확인했다. 실제 키로 생성 요청이나 유료 호출은 하지 않았다. 키체인에 만든 가짜 테스트 항목은 삭제했다. 실행 중인 Switcher는 정상 종료 후 새 빌드로 다시 실행했다. 사용자 Claude/Codex 세션은 종료하지 않았다. 여러 Switcher 백업 앱이 같은 bundle ID로 등록되어 있어 이름만으로 실행하면 구버전이 열리는 것을 확인했고, 최종 실행 파일은 `src-tauri/target/release/bundle/macos/switcher.app/Contents/MacOS/switcher` 경로로 확인했다.

재현 명령:

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml api_connections::tests -- --include-ignored
cargo test --manifest-path src-tauri/Cargo.toml api_connections::activation::tests -- --include-ignored
cargo test --manifest-path src-tauri/Cargo.toml api_connections::balance::tests
node scripts/review-layout.mjs
node scripts/test-workspace-ui.mjs --all-modes docs/design --output docs/four-mode-test-results.json
node scripts/test-workspace-ui.mjs --webkit --output docs/webkit-test-results.json
npm run tauri build -- --bundles app
```

`--include-ignored`는 위 API 모듈에만 한정한다. 해당 로컬 검사는 macOS 키체인에 고유한 가짜 항목을 만들었다 지우고, 설치된 CLI의 버전 또는 인증 상태 명령을 실행한다. 전체 ignored 실계정 검사를 실행하는 명령이 아니다. WebKit 검사는 macOS와 Xcode Command Line Tools가 필요하다.

Windows 자동 검사는 [windows-check](../.github/workflows/windows-check.yml)에서 별도로 실행한다. 코드 푸시 시 Windows 러너에서 프론트 단위 검사·빌드, Rust 전체 검사, 실제 DPAPI 저장·복호화·키 교체·삭제, PowerShell helper 실행, API 해제·구독 전환·활성 연결 삭제 시 OAuth 복원, 설치된 Claude/Codex의 API 인증 인식, 포터블 실행 파일 빌드를 확인한다. CLI 검사는 Claude Code 2.1.300 / Codex 0.167.0과 가짜 키·격리 홈을 사용하며 모델 생성 요청은 하지 않는다. 이 워크플로에는 릴리스·npm 게시 단계가 없다. 문서만 바뀐 푸시는 다시 빌드하지 않는다.

## 이슈와 남은 범위

| 이슈 | 상태·남은 범위 |
| --- | --- |
| [#179 UI](https://github.com/Youkamii/switcher/issues/179) | 요청한 배치·아이콘·4개 모드·숨김 수정 적용. Windows 실화면과 운영 앱의 실제 포인터 상호작용 검증은 남음. |
| [#180 인증](https://github.com/Youkamii/switcher/issues/180) | 오류 구분·재연결·Codex 경로/저장소 보호 적용. CLI 실행 여부로 전체 갱신을 막던 제한 제거. 외부 CLI와 모든 토큰 회전을 원자적으로 조정하는 기능, Codex 네이티브 키링 어댑터는 지원 범위 밖. |
| [#181 API](https://github.com/Youkamii/switcher/issues/181) | 통합 계정 추가, 일반 CLI API 적용·구독 복원. 당시 도입한 월 예산은 11월 20일 로컬 빌드에서 조직 크레딧으로 교체. Windows 실제 DPAPI·PowerShell·설치된 CLI 인증 인식과 구독 복원 검증 완료. 실제 관리자 비용 조회·모델 생성은 미검증. 새 UI 문구는 한국어·영어 제공. |
| [#182 업데이트 서명](https://github.com/Youkamii/switcher/issues/182) | 리뷰에서 확인한 별도 후속 작업. 릴리스 신뢰키·서명 배포 체계는 이번 UI/API 변경에 포함하지 않음. |

이 Mac의 Windows 크로스 컴파일은 SDK 헤더 부재로 중단됐지만, 이후 GitHub Actions의 실제 Windows 러너에서 Rust 전체 검사·DPAPI·PowerShell·CLI 인증 인식을 확인했다. Windows GUI의 실제 포인터 조작 검증은 남아 있다. 기존 macOS Rust 경고 4개는 남아 있으며, 실제 유료 모델 생성은 수행하지 않았다.

## 커밋

- `9963d46` 전체 리뷰·개선 계획·초기 재현 검사
- `b18c7c0` 숨긴 AI 공간·호버 이동 수정
- `b0c0b51` 인증 오류 분류·실패 이유 보존
- `9f21ce4` Codex 경로·인증 저장소 보호
- `672474e` API 보안 저장소·실행 명령 기반
- `1c6c5db` 네 모드 도구 접근성 복원
- `b2ef2c1` API 확인 상태 영속화·명시적 교체·오류 복구·CLI 통합 검사
- `75161f2` 계정 아이콘 배치·API 등록부터 실행까지 연결·Chromium/WebKit 검사
- `f28add7`의 제공자 전체 갱신 차단은 철회. CLI를 사용하면서 다른 계정도 갱신할 수 있어야 한다는 기준으로 회귀 검사를 교체했다.

추가 적용: `fd2b233` — 수동 실행 명령 복사를 일반 CLI API 선택·구독 복원으로 교체하고 API 등록창 드래그 추가. 이후 통합 계정 추가와 API별 사용량·예산을 반영했다.

- `8cdc624` — 구독/API 통합 계정 추가, API별 월 $195 예산·비용 조회/직접 입력, 미조회·초과·월 변경 처리와 회귀 검사.
