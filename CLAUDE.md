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

- 로그인은 브라우저 + 로컬 콜백 방식이다. 코드 붙여넣기 입력칸은 필요 없다 (원격 환경에서만 코드 방식으로 폴백).
- 격리 로그인: `CLAUDE_CONFIG_DIR`(클로드) / `CODEX_HOME`(코덱스)를 임시 폴더로 주면 `.credentials.json`·`.claude.json`·`auth.json`이 전부 그 폴더에만 생성된다 — 활성 계정 무변경.
- **`claude auth login`은 stdin을 리다이렉트하면 즉시 종료된다.** stdin은 상속 그대로 둘 것 (stdout/stderr 리다이렉트는 무해).
- Windows에서는 npm 셔임 때문에 `cmd /c` 경유로 실행하고 `CREATE_NO_WINDOW`로 콘솔 창을 막는다. 취소는 `taskkill /T`로 트리째 — 부모만 죽이면 CLI가 콜백 서버를 문 채 살아남는다.

## 전환 대상 파일 (실측 확인됨)

- 클로드 토큰: `~/.claude/.credentials.json` (키: `claudeAiOauth`)
- 클로드 계정 표시 정보: `~/.claude.json`의 `oauthAccount` 블록 (accountUuid·emailAddress·organizationUuid)
- 코덱스 토큰: `~/.codex/auth.json` (키: `auth_mode`, `OPENAI_API_KEY`, `tokens`, `last_refresh`)
- 프로필 보관소: `~/.switcher/profiles/<name>/`
