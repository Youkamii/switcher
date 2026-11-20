<!-- CLAUDE.md의 사본 (codex CLI용) — CLAUDE.md를 고치면 이 파일도 같이 갱신할 것 -->

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
- **앱이 비활성이면 WKWebView 페이지가 `visibilityState=hidden`이 될 수 있다** (위젯은 비활성 패널이라 상시 해당). hidden 페이지는 rAF 완전 정지·타이머 ≥0.5~1초 지연이고, 정지가 겹치면 setTimeout도 사실상 죽는다 — **해제·종료 같은 필수 동작을 웹뷰 타이머·rAF 완료 콜백에 걸지 말 것** (#52 실측: 흔들기 해제 연출이 얼며 검은 화면 고착). 대책 패턴: 웹뷰는 감지 즉시 invoke(예약)만 하고, 확실한 마무리는 러스트가 진다. 또한 비활성 앱의 창은 마우스 이벤트 자체를 못 받으므로 전역 폴링(CGEventCreate 커서 좌표 — 권한 불필요) 기반 네이티브 백업이 필요하다 (lib.rs `ShakeTracker`).

## API 연결 (구독 프로필과 별도)

- `api_connections.rs`: macOS Keychain / Windows 사용자 DPAPI에 키 저장. 메타데이터·런처·로그에 키 원문 금지.
- API는 사용자가 **저장하고 사용/사용**을 누르면 일반 CLI 설정에 적용한다. Claude는 `apiKeyHelper`, Codex는 공식 공급자 설정 + 활성 `auth.json`(Unix 0600). 적용 전 OAuth·설정 백업, 구독 전환/해제/삭제 시 앱이 소유한 설정만 복원한다. API 선택은 TFSD를 해제하며 자동 유료 전환은 금지한다.
- Codex 구독·API 어댑터는 파일 저장소만 지원한다. `keyring`/`auto`/`ephemeral` 설정이면 전환·쓰기 차단.
- 화면 회귀 검사: `npm run build && node scripts/test-workspace-ui.mjs`, 배치 검사: `node scripts/review-layout.mjs`. 가짜 데이터만 사용한다.
- 계정 추가 창에서 구독 로그인/API 키 선택을 통합한다. 별도 API 등록 진입 버튼을 만들지 않는다. `api_connections/balance.rs`는 Claude API 키로 확인한 조직과 로그인 프로필의 계정·조직을 대조한 뒤 선불 잔액을 읽는다. 같은 조직의 키는 잔액을 공유한다. 자동 조회가 어려운 경우 한국어·영어 Console 대시보드의 조직 크레딧을 가져와 출처와 시각을 표시한다. 대시보드 원문은 IPC·디스크로 보내지 않는다. OpenAI는 공식 결제 화면을 제공한다. 월 예산·관리자 조회 키 입력은 제거했다.
- API 사용법·검증 한계: `docs/API_CONNECTIONS.md`, `docs/IMPLEMENTATION_STATUS.md`.
