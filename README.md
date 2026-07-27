# switcher

Claude Code · Codex CLI 다중 계정 전환 위젯 (Windows)

여러 AI 계정을 쓸 때 매번 `/login`으로 재로그인하는 대신, 위젯에서 버튼 한 번으로 계정을 전환하고 계정별 사용량을 한눈에 봅니다.

A desktop widget that switches between multiple Claude Code / Codex CLI accounts in one click and shows per-account usage.

## 동작 원리

- 클로드·코덱스는 로그인 토큰을 로컬 파일에 저장합니다.
  - Claude Code: `~/.claude/.credentials.json`
  - Codex CLI: `~/.codex/auth.json`
- switcher는 계정별 토큰 파일을 프로필로 보관하고, 전환할 때 두 단계로 교체합니다.
  1. 지금 활성 파일을 현재 계정 프로필에 백업 (토큰이 수시로 갱신되므로 필수)
  2. 대상 계정 프로필 파일을 활성 위치로 복사
- 메모리·설정·대화 기록은 계정과 무관한 로컬 폴더에 있어, 계정을 바꿔도 작업 환경이 유지됩니다.
- 이미 실행 중인 CLI 세션은 기존 토큰을 계속 쓰고, 새로 여는 세션부터 전환된 계정이 적용됩니다.

## 계정 추가 방법

위젯은 OAuth 로그인을 대신할 수 없습니다 (브라우저 인증이 필요). 계정마다 처음 한 번만 CLI 로그인이 필요합니다.

1. 지금 로그인된 계정을 위젯에서 이름 붙여 저장
2. 터미널에서 `claude` 실행 후 `/login` (코덱스는 `codex login`) 으로 다른 계정 로그인
3. 위젯에 뜨는 "저장 안 된 계정" 안내에 따라 새 이름으로 저장
4. 이후로는 위젯 버튼 한 번으로 전환 — 재로그인 불필요

## 알려진 한계

- 이미 켜져 있는 CLI 세션은 메모리에 든 기존 토큰을 계속 쓰므로, 전환은 새로 여는 세션부터 적용됩니다.
- 실행 중인 클로드 세션이 토큰을 자동 갱신하면 방금 전환한 계정의 활성 파일을 덮어쓸 수 있습니다. 전환은 CLI 작업이 뜸한 때에 하는 것이 안전합니다. (프로필 보관함에는 한 세대 `.bak` 백업이 남습니다)
- 사용량 API에는 요청 제한이 있어 60초 캐시로 조회합니다.

## 개발

```sh
npm install
npm run tauri dev
```

포터블 빌드:

```sh
npm run tauri build -- --no-bundle
# → src-tauri/target/release/switcher.exe
```

## 라이선스

미정 (완성 후 MIT 예정)
