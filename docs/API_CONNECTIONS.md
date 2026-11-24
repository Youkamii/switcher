# 계정 전환 범위와 API 키 기능 종료

> **API 키 기능은 v2.2.3에서 뺐다** (#191). 등록·사용·잔액 표시, 미니멀·엣지의 「한 번 더 = API 키」, 다른 PC로 옮길 때의 키 이동이 모두 없어졌다. 위젯은 구독 계정만 다룬다.

## v2.2.2 이하에서 올라올 때

- **API 키로 켜 둔 상태였다면** 새 버전이 처음 뜰 때 구독으로 되돌린다. 화면에 끌 곳이 없어졌기 때문이다. Claude는 `settings.json`의 `apiKeyHelper`와 앱이 넣은 값만 걷어내고, Codex는 API를 켜기 전의 `auth.json`·`config.toml`을 복원한다. 계정을 전환할 때도 같은 복원을 한다.
- **저장해 둔 키는 지우지 않는다.** 화면에만 나오지 않는다. 키는 macOS 키체인(`Switcher-API-<공급자>-<키 이름>`)·Windows 현재 사용자 DPAPI에, 이름과 확인 기록은 `~/.switcher/api-connections/`에 그대로 있다.
- 완전히 지우려면: macOS는 **키체인 접근**에서 `Switcher-API-`로 시작하는 항목을 지우고 `~/.switcher/api-connections` 폴더를 지운다. Windows는 `%USERPROFILE%\.switcher\api-connections` 폴더를 지우면 키도 함께 없어진다(키가 그 폴더 안에 암호화돼 있다). 공급자 쪽 키 폐기는 발급 페이지에서 따로 한다.
- 다른 PC로 옮기기는 구독 로그인만 옮긴다. v2.2.x가 만든 글·파일도 받을 수 있지만 그 안의 API 키는 읽지 않는다.

## 플랜의 월간 API 크레딧

Max·Ultra·Team 플랜에 포함된 월간 API 크레딧은 터미널의 대화형 Claude Code에 쓰이지 않는다. 직접 API 호출, Console Playground, Managed Agents, Agent SDK, 그리고 연결된 Console 조직의 키로 직접 돌리는 `claude -p`가 대상이다 ([Anthropic 안내](https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans)). 이것이 위젯에서 API 전환을 뺀 이유다 — 대화형 세션을 그 키로 넘기면 `Credit balance too low`만 났다.

위젯 없이 쓰려면 키를 그 실행에만 준다. 대화형 세션은 구독 그대로 돈다.

```bash
ANTHROPIC_API_KEY="<키>" claude -p '시킬 일'
```

이전 버전에서 저장해 둔 키가 있으면 macOS에서는 `"$(security find-generic-password -s 'Switcher-API-claude-<키 이름>' -w)"`로 꺼내 쓸 수 있다. 시킬 말은 따옴표로 감싼다(zsh는 따옴표 없는 `?`·`*`를 파일 이름 찾기로 읽는다). 실측(2026-11-22·24, macOS, Claude Code 2.1.330·2.1.331): 짧은 요청이 이 방식으로 응답했고, 도구를 쓰는 긴 작업과 Windows는 확인하지 않았다.

## Codex 계정 전환 범위

위젯은 현재 인증을 먼저 백업하고 선택한 계정의 `auth.json`을 적용한다. **파일 교체만으로 실행 중 인증이 바뀌지는 않으므로 공유 서버 갱신을 이어서 수행한다.** Codex CLI 0.167.0은 인증을 메모리에 유지하며 외부 파일 변경을 자동으로 읽지 않는다. 401 복구 시에도 다른 계정으로의 재읽기를 허용하지 않는다. [해당 버전 AuthManager 구현](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/login/src/auth/manager.rs).

구독 계정 전환 뒤에는 **Codex 0.167.0 이상 정식 버전의 관리되는 공유 서버**를 자동 갱신한다. 실행 중인 작업·승인 대기를 기다리고 저장 가능한 대화가 모두 idle일 때 정상 종료를 요청한다. 서버 종료를 확인한 뒤 공식 daemon 명령으로 다시 시작하고, 새 인증이 읽혔는지 확인한다. 기존 Codex TUI의 재연결 기능으로 대화를 이어가는 방식이며 터미널 입력을 주입하지 않는다. Codex 자체 OAuth 갱신을 유지하므로 별도의 외부 토큰 공급자에 의존하지 않는다.

화면의 **선택됨**과 공유 서버 갱신 상태는 별개다. `--no-daemon`, `--profile`, 일반 설정을 덮어쓰는 `-c` 옵션이나 이를 넣는 래퍼, 구형 CLI, Windows 관리자 실행, 독립 stdio 서버처럼 공유 서버를 사용하지 않는 실행은 자동 갱신 대상이 아니다. 이 경우 기존 Codex를 종료한 뒤 같은 터미널에서 `codex resume`으로 이어간다.

갱신 작업은 별도 headless helper가 수행하므로 위젯을 닫아도 이어진다. Windows에서는 설치 EXE를 잠그지 않는 검증된 복사본을 사용한다. 이미 진행 중인 작업을 강제 종료하지 않으며, 30분간 idle이 되지 않으면 실패 상태로 끝나고 새로고침으로 재시도할 수 있다. 정상 종료 요청 직후 새 작업이 시작된 경합에서는 Codex의 drain이 끝날 때까지 helper가 기다린다. 저장되지 않는 ephemeral 대화는 재시작하지 않는다.

`node scripts/check-codex-session-auth.mjs`는 격리된 홈·가짜 OAuth 계정 두 개로 설치된 Codex를 실행한다. 기존 프로세스는 A, 파일 교체 후 새 프로세스는 B를 읽는지 검사한다. 생성 요청이나 실계정 토큰을 사용하지 않는다. 이 검사는 로그인 인식 범위이며 실제 모델 호출 성공을 뜻하지 않는다.

`cargo test --manifest-path src-tauri/Cargo.toml installed_codex_daemon_reloads_selected_account_without_external_auth -- --ignored --test-threads=1`은 설치된 packaged Codex와 격리 홈·가짜 인증·로컬 HTTP 모델 응답을 쓴다. 작업 중 갱신 대기, 완료 후 B 인증 반영, 같은 thread ID와 대화 내용 복원, managed OAuth 유지까지 확인한다. 실제 OpenAI 모델 호출이나 유료 요청은 하지 않는다. 이 반복 검사는 앱 서버 수준에서 재연결·resume을 수행한다. macOS Codex 0.169.0에서는 별도로 격리 PTY의 실제 TUI와 로컬 HTTPS 응답을 사용해 자동 재연결 후 같은 대화에서 후속 입력이 처리되고 이전 답변이 문맥에 유지되는 것까지 확인했다.

## 인증 오류 처리

| 표시 | 처리 |
| --- | --- |
| 구독 액세스 토큰 갱신 필요 | 해당 계정 CLI를 실행한 뒤 사용량 새로고침 |
| 저장된 갱신 토큰 거부 | 계정 카드의 **다시 연결**에서 같은 계정 로그인 |
| 권한 거부 | 계정·프로젝트·조직 접근 정책 확인 |
| 요청 제한·네트워크 실패 | 잠시 후 새로고침 |

Claude/Codex 실행 여부만으로 다른 계정의 토큰 갱신을 막지 않는다. 활성 OAuth 계정은 중복 회전하지 않는다.
