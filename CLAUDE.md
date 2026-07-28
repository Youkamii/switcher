# switcher

클로드 코드·코덱스 CLI 다중 계정 관리 위젯 (Windows). Public repo — 비밀 유출에 특히 민감.

## 스택 (고정 — 다른 도구 임의 제안 금지)

- Tauri 2 + Rust (계정 전환·사용량 조회는 전부 Rust 커맨드)
- 프론트: 바닐라 TypeScript + Vite (프레임워크 없음)
- 패키지 매니저: npm
- 테스트: `cargo test` (src-tauri에서), 실행 검증은 tauri dev/build

## 실검증 명령

- 개발 실행: `npm run tauri dev`
- 포터블 빌드: `npm run tauri build -- --no-bundle` → `src-tauri/target/release/switcher.exe`
- Rust 빠른 검증: `cd src-tauri && cargo check`

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

- 클로드 토큰: `~/.claude/.credentials.json` (키: `claudeAiOauth`)
- 클로드 계정 표시 정보: `~/.claude.json`의 `oauthAccount` 블록 (accountUuid·emailAddress·organizationUuid)
- 코덱스 토큰: `~/.codex/auth.json` (키: `auth_mode`, `OPENAI_API_KEY`, `tokens`, `last_refresh`)
- 프로필 보관소: `~/.switcher/profiles/<name>/`
