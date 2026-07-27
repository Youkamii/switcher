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
