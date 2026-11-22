# switcher

클로드 코드·코덱스 CLI 다중 계정 관리 위젯 (Windows·macOS). Public repo — 비밀 유출에 특히 민감.

## 스택 (고정 — 다른 도구 임의 제안 금지)

- Tauri 2 + Rust (계정 전환·사용량 조회는 전부 Rust 커맨드)
- 프론트: 바닐라 TypeScript + Vite (프레임워크 없음)
- 패키지 매니저: npm
- 테스트: `cargo test` (src-tauri에서), 실행 검증은 tauri dev/build

## 실검증 명령

- 개발 실행: `npm run tauri dev`
- 포터블 빌드 (윈도우): `npm run tauri build -- --no-bundle` → `src-tauri/target/release/switcher.exe`
- 앱 빌드 (맥): `npm run tauri build -- --bundles app` → `src-tauri/target/release/bundle/macos/switcher.app`
- Rust 빠른 검증: `cd src-tauri && cargo check`
- 실계정 e2e (로컬 전용): `cargo test -- --ignored real_ --test-threads=1`

## 금기

- 토큰·프로필·계정 파일 커밋 절대 금지 (public repo). `.gitignore`의 Secrets 블록을 유지하고, 테스트 픽스처에도 실토큰 사용 금지.
- **전환 순서 불변**: 활성 파일을 현재 프로필에 먼저 백업 → 그다음 대상 프로필 복사. 토큰이 자동 갱신되므로 순서를 바꾸면 최신 토큰이 유실된다.
- 로그·에러 메시지에 토큰 값 출력 금지.
- 새 터미널 창 스폰 금지.

## 로그인 (실측 확인됨)

- **출력을 파이프로 받으면 CLI가 화면을 그리지 않는다.** `claude auth login`은 "Opening browser to sign in…" 한 줄만 내보낸다. 진짜 콘솔(PTY)을 붙여야 로그인 주소와 `Paste code here if prompted >` 프롬프트가 나온다. 파이프로 확인하고 "기능이 없다"고 단정하지 말 것 — 실제로 그 실수를 했다.
- PTY를 붙여도 **`ESC[6n`(커서 위치 질의)에 `ESC[1;1R`로 답하지 않으면** 화면이 그려지지 않는다.
- TUI는 줄바꿈 대신 커서 이동으로 그리므로, ANSI 제거 시 색상(SGR, 최종 바이트 `m`)만 버리고 **나머지 CSI는 줄바꿈으로 치환**해야 글자가 붙지 않는다.
- 코덱스는 `codex login --device-auth`가 주소와 일회용 코드를 글자로 준다 (브라우저를 열지 않음).
- 격리 로그인: `CLAUDE_CONFIG_DIR`(클로드) / `CODEX_HOME`(코덱스)를 임시 폴더로 주면 `.credentials.json`·`.claude.json`·`auth.json`이 전부 그 폴더에만 생성된다 — 활성 계정 무변경.
- Windows에서는 npm 셔임 때문에 `cmd /c` 경유로 실행한다. 취소는 `taskkill /T`로 트리째 — 부모만 죽이면 CLI가 살아남는다. 임시 폴더 삭제는 종료 직후 실패할 수 있으니 재시도할 것.

## 전환 대상 파일 (실측 확인됨)

- 클로드 토큰 (윈도우): `~/.claude/.credentials.json` (키: `claudeAiOauth`)
- 클로드 토큰 (맥): **키체인** 항목 `Claude Code-credentials` (계정=사용자명, JSON 내용은 윈도우와 동일). `~/.claude/.credentials.json`은 구버전 잔재 — 위젯은 키체인을 정본으로 쓰고, 파일이 남아 있으면 함께 갱신한다.
- 클로드 계정 표시 정보: `~/.claude.json`의 `oauthAccount` 블록 (accountUuid·emailAddress·organizationUuid) — 맥에서도 동일
- 코덱스 토큰: `${CODEX_HOME:-~/.codex}/auth.json` (키: `auth_mode`, `OPENAI_API_KEY`, `tokens`, `last_refresh`) — 맥에서도 파일 그대로
- Codex는 실행 중 인증을 메모리에 보관한다. 파일 교체만으로는 바뀌지 않으므로 `codex_session.rs`가 구독 전환 후 Codex 0.167.0 이상 정식 버전의 관리되는 공유 서버를 갱신한다. 작업·승인 대기·ephemeral 대화는 건드리지 않고, idle이면 정상 종료 → 종료 확인 → 공식 daemon 재시작 → 인증 확인 순서로 진행한다. 외부 토큰 모드나 강제 종료 금지. helper는 위젯 종료 후에도 마무리한다. 단독 실행·일반 `-c` 설정 주입 래퍼·`--profile`·구형 CLI·Windows 관리자 실행은 다음 실행/`codex resume` 대상이다. 기본 재현: `node scripts/check-codex-session-auth.mjs`; 공유 서버 회귀: `cargo test installed_codex_daemon_reloads_selected_account_without_external_auth -- --ignored --test-threads=1`.
- 프로필 보관소: `~/.switcher/profiles/<name>/`

## macOS 전용 (실측 확인됨, 2026-07-29 / claude 2.1.220 / macOS 26.5)

- **claude CLI 버전별 자격증명 저장 변천 (실측 2026-08-07)**: ~2.1.220은 키체인 raw JSON. 2.1.221~222는 키체인에 **hex 문자열**로 쓰기도 함(읽기는 양쪽 관용) — 위젯 읽기 관문(accounts.rs `normalize_cred`)이 순수 hex→JSON일 때만 투명 디코드. **2.1.223+는 네이티브 키체인 API로 전환** — 위젯이 security 도구로 쓴 항목을 ACL 불일치로 읽지 못해 "Not logged in"이 된다. 대신 `~/.claude/.credentials.json` 파일이 있으면 읽어 자기 소유로 키체인에 이관하고 파일을 지운다. → **위젯 전환은 키체인(raw JSON)과 파일을 둘 다 쓴다** (구형 CLI는 키체인, 신형 CLI는 파일 경유로 각자 동작).
- 미확인: 2.1.223의 격리 로그인(CLAUDE_CONFIG_DIR)이 키체인 접미사 항목 대신 임시 폴더 파일을 남기는지 — 다음 "계정 추가" 검증 때 확인할 것 (login.rs 임포트 경로에 영향).

- **관리자 승인(do shell script) 컨텍스트에서 `/usr/bin/nohup` 즉사** (실측 2026-08-12, macOS 26.5): 제어 터미널이 없어 `nohup: can't detach from console: Inappropriate ioctl for device`로 명령 실행 전에 죽는다. root 백그라운드는 순수 `&` + 본문 `trap '' HUP`으로 띄울 것 — 승인 셸 종료 후 launchd 재입양으로 계속 도는 것 실측 확인 (clamshell.rs arm_command). 이 때문에 v1.7.35까지는 클램셸 감시자가 이 맥에서 한 번도 뜬 적이 없었다 (구버전은 생존 검증이 없어 성공처럼 보였음).
- 키체인 접근은 claude CLI와 같은 통로인 `/usr/bin/security`를 쓴다 — 같은 통로여야 항목 ACL이 일치해 허용 팝업이 없다. 쓰기는 `security -i`(stdin) + `-X`(hex)로 — 토큰이 프로세스 인자에 노출되지 않는다.
- 격리 로그인(`CLAUDE_CONFIG_DIR`)은 맥에서 파일 대신 키체인 항목 `Claude Code-credentials-<sha256(경로 문자열)[:8]>`을 만든다. 청소는 **키체인 먼저, 폴더 나중** — 폴더가 사라지면 항목 이름(경로 해시)을 복원할 수 없다.
- 일반 NSWindow는 `CanJoinAllSpaces`·`FullScreenAuxiliary`를 줘도 다른 Space(특히 전체화면)에 올라가지 못한다 — **비활성 NSPanel로 클래스를 갈아끼워야** 한다 (lib.rs `SwitcherPanel`). 이때 tao가 덮어쓰던 `canBecomeKeyWindow`가 사라지므로 서브클래스에서 복원해야 입력칸이 포커스를 받는다.
- GUI 앱(Finder 실행)은 셸 PATH를 모른다 — CLI 경로는 로그인 셸(`zsh -lc command -v`)과 관례 경로(`~/.local/bin` 등)로 해석한다 (login.rs `resolve_program`).
- 마우스 전역 상태는 `CGEventSourceButtonState`(권한 불필요), 더블클릭 간격은 `NSEvent.doubleClickInterval`.
- 내장 패널 밝기 0(DisplayServices) = **백라이트 완전 소등** — 오버레이·연기 연출·커서까지 화면 전체가 안 보인다 (입력은 살아 있어 ESC는 동작). 그래서 블랙 모니터의 밝기 최하 연동(#49)은 Windows 전용이고 맥은 오버레이만 쓴다 (#51, 사용자 실측 v1.7.21).
- **Type4 벽 붙임 맥 실측 (2026-09-30, #152 / macOS 27)**: NSPanel에서 `set_shadow(false)`, 러스트 커서 폴링(`mouseLocationOutsideOfEventStream`)의 edge-hover, SVG goo 필터(+SMIL `beginElement`는 보이는 창의 WKWebView 별도 실험으로만 확인, 위젯 내부 블러 전이는 미관측), 좌/우 벽 스냅(테두리 없는 창이라 insets 0), ☰ 드래그 후 반대 벽 갈아타기, 꾹 누르기(0.25s) 전부 동작. 멀티모니터 벽 갈아타기만 미실측(단일 디스플레이).
- **맥 시스템 글꼴(SF)은 숫자가 Pretendard·맑은 고딕보다 넓다** — 9px 굵게 "23:59"가 26.6px(윈도우 쪽 주석의 추정치 ≈19px — 측정값 아님). 좁은 열의 글자 폭은 Windows 실측값을 믿지 말고 WKWebView로 재서 `body.mac`(main.ts가 `navigator.platform`으로 붙임)에 따로 맞춘다.
- **다른 창을 클릭한 뒤 위젯의 첫 클릭이 매번 한 번 삼켜지던 문제는 창 설정 `acceptFirstMouse: true`로 해결** (tauri.conf.json, 2026-09-30 #152). 원인은 비활성 창의 첫 클릭을 WKWebView가 활성화용으로 먹던 것(wry `acceptsFirstMouse` 기본 false). 이 때문에 Type4 ☰ 드래그가 첫 시도에 안 잡혔다. 실입력 E2E의 "선클릭 1회" 요령은 이제 불필요.
- **☰ 드래그 중 잠깐 멈추면 moved 이벤트가 180ms 끊겨 "이동 끝"으로 오판**해 Type4가 가까운 벽으로 되돌리던 문제 → `pointer_button_down`(전역 버튼 상태)으로 버튼을 놓을 때까지 정착을 미룬다 (main.ts `settleMonitorSync`). 화면 중앙을 넘겨 놓아야 반대편 벽에 붙는 규칙은 그대로.
- **앱이 비활성이면 WKWebView 페이지가 `visibilityState=hidden`이 될 수 있다** (위젯은 비활성 패널이라 상시 해당). hidden 페이지는 rAF 완전 정지·타이머 ≥0.5~1초 지연이고, 정지가 겹치면 setTimeout도 사실상 죽는다 — **해제·종료 같은 필수 동작을 웹뷰 타이머·rAF 완료 콜백에 걸지 말 것** (#52 실측: 흔들기 해제 연출이 얼며 검은 화면 고착). 대책 패턴: 웹뷰는 감지 즉시 invoke(예약)만 하고, 확실한 마무리는 러스트가 진다. 또한 비활성 앱의 창은 마우스 이벤트 자체를 못 받으므로 전역 폴링(CGEventCreate 커서 좌표 — 권한 불필요) 기반 네이티브 백업이 필요하다 (lib.rs `ShakeTracker`).

- **비활성 패널은 WKWebView 마우스 이동(:hover·pointermove)을 받지 못한다** (WebKit 소스 확인 2026-10-07: `Source/WebKit/UIProcess/mac/WebViewImpl.mm` `trackingAreaOptions()`가 `NSTrackingActiveInKeyWindow` — 시스템 설정이 레거시 스크롤바(마우스 연결 등)일 때만 `ActiveAlways`). 위젯 패널은 `becomesKeyOnlyIfNeeded`라 입력칸을 누를 때만 키 윈도우가 되므로 Type1의 CSS :hover는 맥에서 사실상 오지 않는다. 그래서 Type1 호버 버튼(#103)은 러스트 커서 폴링(`set_hover_poll` → `hover-poll {idx,x,y}` → main.ts `applyPolledHover`가 `.hit-hover` + 합성 pointerenter/leave)으로 만든다 — Type2/3의 `card-hover`, Type4의 `edge-hover`와 같은 방식. 클릭(`acceptFirstMouse`)·키보드는 그대로 온다.

## 보기 모드 (사용자 지시 2026-11-21~22)

- 보기 모드는 **1 일반 / 2 미니멀 / 3 엣지** 셋이다. 2 컴팩트는 없앴다(#183) — 다시 제안하지 말 것. 이 문서와 코드 주석의 옛 이름은 Type1 = 일반, Type2(컴팩트) = 폐지, Type3 = 미니멀, Type4 = 엣지로 읽는다. 코드의 `compact*` 이름은 "축소 레이아웃"이라는 뜻으로 미니멀이 쓴다.
- 엣지 손잡이 높이(#185): 패널이 길어 창을 더 올리거나 내릴 수 없을 때 ☰를 화면 끝 너머로 끌면 넘친 만큼 손잡이(`--edge-tab-y`, localStorage `switcher.edgetab`)가 옮겨진다. 위쪽 끝은 창 좌표로는 넘침을 알 수 없어 `pointer_position` 커맨드로 잰다.

## 위젯 모드의 구독 ↔ API 전환 (#186, 실측 2026-11-22)

- 규칙(사용자 지시): 한 번 = 그 계정(구독), 사용 중인 계정을 한 번 더 = 그 계정의 API 키, **다시 한 번 = 구독으로 복귀**. 엣지는 클릭, 미니멀은 더블클릭(러스트 감지)이 "한 번"이다. 토글은 되돌아오는 길까지가 한 기능이다 — 복귀를 빼먹거나 조건을 달지 말 것.
- **API를 쓰는 중인 계정은 틀(`.account-with-api`) 전체가 복귀 대상**이다. 전환 직후 포인터 밑은 계정 줄이 아니라 API 칸으로 바뀐다. 계정 카드만 받으면 같은 자리를 다시 눌러도 돌아오지 않는다(실제로 사용자가 겪은 결함).
- 판정은 누름 시각으로 하고 `event.detail`(클릭 횟수)은 쓰지 않는다. 기준은 `src/apiToggle.ts`(엣지 클릭)와 `lib.rs` `api_click_allowed`(미니멀 더블클릭) 두 곳에 같은 값(같은 동작 0.6초, 화면 반영 0.25초)으로 있다 — 바꿀 때는 둘 다. 막는 것은 유료(API) 쪽뿐이고 복귀는 막지 않는다. 휴지를 "전환이 끝난 시각"부터 재면 0.7초 박자의 누름이 삼켜진다 — 누름 시각부터 잰다.
- 전환 뒤 화면 교체는 사용량 조회를 기다리는 스무스 렌더다. 전환 직후에 필요한 표식(`API ›`, 히트 영역)은 낙관적으로 바로 단다(`offerApiOptimistic`). 교체 렌더를 기다리게 하면 "한 번 더"가 먹지 않는 구간이 생긴다.
- `.api-owner`는 키 등록 창의 소속 문단 클래스다(글자 크기·프라이버시 블러가 걸려 있다). 위젯 카드에는 `owns-api`를 쓴다.
- API 상태 색은 테마별 `--api-rgb`(계정색·코덱스색·경고색과 겹치지 않게 고른 값)다. 호버에서도 이 색을 유지한다 — 누른 직후 포인터가 그대로 올라가 있으므로, 호버가 색을 바꾸면 전환이 안 된 것처럼 보인다.
- **Claude Code는 API 선택·해제를 실행 중인 세션에도 바로 적용한다** (2.1.328 실측: 구독으로 돌던 세션이 선택 직후의 요청에서 `billing_error` "Credit balance is too low"). 그러므로 사용자의 실제 위젯에서 사용 중인 계정 카드를 시험 삼아 누르지 말 것 — 실행 중인 세션(이 작업 세션 포함)이 유료 키로 넘어간다. 실제 포인터 검증은 아래 격리 실행으로만 한다.

- **플랜의 월간 API 크레딧(Max·Ultra·Team)은 대화형 Claude Code에 쓰이지 않는다** (Anthropic 안내, 2026-11-22 확인 — 대상은 직접 API 호출·Console Playground·Managed Agents·Agent SDK·`claude -p`). Console과 위젯에 금액이 보여도 대화형 세션을 API 키로 넘기면 `Credit balance is too low`가 날 수 있고, 모델과는 무관하다. 이 오류를 "잔액이 없다"로 옮겨 설명하지 말 것 — 위젯에 금액이 보이는 사용자에게 그렇게 답해서 틀렸다. 위젯이 저장한 금액(`api-connections/<provider>/<name>/console-credit.json`)과 플랜을 먼저 대조한다.
- **월간 크레딧은 `claude -p`로 쓴다** (실측 2026-11-22, 2.1.330, 사용자 승인하에 과금 호출): `ANTHROPIC_API_KEY`를 환경변수로 준 `claude -p`는 Haiku 5.5·Fable 5.2 모두 응답했다. `-p`에서는 구독 로그인보다 환경변수 키가 먼저다(가짜 키로 401 확인 — 과금 없이 볼 수 있다). 바이너리는 요청마다 청구 표시(`cc_entrypoint`)를 보내며 `-p`는 `sdk-cli`, 대화형은 `cli`다. 위젯의 API 전환(`apiKeyHelper`)은 대화형까지 키로 넘기므로 월간 크레딧만 있는 조직에는 맞지 않는다. 실키로 하는 호출은 금액과 함께 물어 승인받은 뒤에만, 깨끗한 환경(`env -i`)에서 키를 화면·인자에 내지 않고 한다.

## 격리 실행 검증 (macOS, 실측 2026-11-22)

- 실제 앱의 포인터 동작은 가짜 홈에서 확인한다. `HOME`과 `CFFIXED_USER_HOME`을 임시 폴더로 주면 `Env::real()`과 `/usr/bin/security`가 그 안만 본다. 그 홈에 키체인을 새로 만든다: `security create-keychain -p "" <홈>/Library/Keychains/login.keychain-db` → `default-keychain -s` → `list-keychains -d user -s` → `unlock-keychain -p ""` → `set-keychain-settings` (전부 `HOME=<홈>`으로). 합성 프로필·합성 키만 넣고 `sandbox-exec -p '(version 1)(allow default)(deny network-outbound (remote ip))'`로 번들 실행 파일을 직접 띄운다. `SWITCHER_VIEW=edge|minimal|normal`, `SWITCHER_DIAG=1`.
- 단일 인스턴스라 사용자의 실제 위젯을 먼저 종료하고, 끝나면 `open -a`로 다시 띄운다. 전후로 실제 `~/.claude/settings.json`·`~/.codex/auth.json`의 해시와 기본 키체인이 그대로인지 확인한다.
- 화면이 잠겨 있으면 키체인 unlock이 실패하고 합성 클릭은 잠금 화면으로 간다 — 잠금이 풀린 뒤에만 할 수 있다.
- 위젯의 실제 localStorage는 `~/Library/WebKit/com.youkamii.switcher/…`에 있다. `~/Library/WebKit/switcher/…`는 옛 개발 실행의 잔재다.
- 이 맥(macOS 27.2 SDK)에서는 `clang` 링크가 실패한다(tapi `arm64e.x1`) — 보조 도구는 `swiftc`로 만든다. `screencapture -R`은 간헐적으로 실패하므로 전체를 찍고 `sips`로 자른다.
- `scripts/test-workspace-ui.mjs --all-modes <폴더>`는 바로 다음 인자를 출력 폴더로 받는다. `--all-modes --output x`라고 쓰면 저장소에 `--output/` 폴더가 생긴다.

## 배포·서명 (사용자 결정 2026-10-02)

- 코드 서명·공증은 하지 않는다 — 유료 계정(코드 서명 인증서, Apple Developer Program)을 쓰지 않기로 결정. 다시 제안하지 말 것. 첫 실행 경고는 README 설치 절의 NOTE(Windows "추가 정보 → 실행", macOS "그래도 열기" 또는 `xattr -dr com.apple.quarantine`)로 안내한다.
- 실측(2026-10-02): Node(fetch + fs.writeFileSync)로 받은 zip과 자체 압축 해제한 exe에는 Zone.Identifier(Mark of the Web)가 없다 → npm 설치 경로는 SmartScreen이 걸리지 않는다. 경고는 브라우저로 받은 파일에만 뜬다. macOS quarantine도 같은 원리이지만 실기기로는 미확인.

## 계정별 API 키

- `api_connections.rs`: macOS Keychain / Windows 사용자 DPAPI에 키 저장. 메타데이터·런처·로그에 키 원문 금지.
- API는 사용자가 **저장하고 사용/사용**을 누르면 일반 CLI 설정에 적용한다. 위젯 모드에서 사용 중인 계정 카드를 한 번 더 누르는 것도 같은 명시적 선택이다(#186, 위 절). Claude는 `apiKeyHelper`, Codex는 공식 공급자 설정 + 활성 `auth.json`(Unix 0600). 적용 전 OAuth·설정 백업, 구독 전환/해제/삭제 시 앱이 소유한 설정만 복원한다. API 선택은 TFSD를 해제하며 자동 유료 전환은 금지한다.
- Codex 구독·API 어댑터는 파일 저장소만 지원한다. `keyring`/`auto`/`ephemeral` 설정이면 전환·쓰기 차단.
- 화면 회귀 검사: `npm run build && node scripts/test-workspace-ui.mjs`, 배치 검사: `node scripts/review-layout.mjs`. 가짜 데이터만 사용한다.
- 일반 모드의 **계정 추가**는 로그인을 시작한다. 새 API 키는 저장된 계정 카드의 **API 키 추가**에서만 등록한다. 소속은 공급자와 프로필의 영구 ID로 기록하고 키 이름은 계정 안에서 구분한다. 기존 키는 사용자가 고른 계정에 연결한다. 이때 OS 보안 항목, 키 교체 버전, 잔액 기록과 활성 CLI 설정을 보존한다.
- `api_connections/balance.rs`는 실행 키로 조직을 확인한다. 위젯 로그인은 소속 계정과 발급 조직이 일치할 때 사용하고, Console 로그인은 발급 조직을 확인한다. 같은 조직의 키는 잔액을 공유한다. Console에서 가져온 크레딧에는 출처와 확인 시각을 표시하고 대시보드 원문은 저장하지 않는다. OpenAI는 공식 결제 화면을 제공한다.
- API 사용법·검증 한계: `docs/API_CONNECTIONS.md`, `docs/IMPLEMENTATION_STATUS.md`.
