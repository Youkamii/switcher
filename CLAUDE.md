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

## 전환 대상 파일 (실측 확인됨)

- 클로드 토큰: `~/.claude/.credentials.json` (키: `claudeAiOauth`)
- 클로드 계정 표시 정보: `~/.claude.json`의 `oauthAccount` 블록 (accountUuid·emailAddress·organizationUuid)
- 코덱스 토큰: `~/.codex/auth.json` (키: `auth_mode`, `OPENAI_API_KEY`, `tokens`, `last_refresh`)
- 프로필 보관소: `~/.switcher/profiles/<name>/`
