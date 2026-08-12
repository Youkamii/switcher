# macOS 검증 에이전트 전달 프롬프트

`switcher`의 Windows 쪽 정적 수정은 끝났고, 이제 실제 Apple Silicon Mac에서 macOS 전용 동작을 검증해야 한다. 아래 범위만 확인하라.

## 절대 경계

- 현재 작업 트리의 변경을 보존하라. `reset`, `checkout --`, stash, 임의 되돌리기를 하지 마라.
- 커밋·푸시·태그·GitHub 릴리스·npm 게시는 하지 마라.
- 실계정 토큰 파일, 키체인 내용, 이메일, 카카오톡 등 개인정보를 출력·복사·촬영하지 마라. `~/.claude`, `~/.codex`, `~/.switcher/profiles`의 내용은 열지 마라.
- 기본 테스트에서는 ignored 실계정 e2e를 실행하지 마라. 필요하면 먼저 사용자 허락을 받아라.
- 화면 크기·CSS·밝기 바 등 이번 범위 밖 UI를 수정하지 마라.
- 재부팅과 수동 `sudo pmset` 복구는 반드시 사용자에게 먼저 허락받아라.
- 확인되지 않은 것을 성공이라고 쓰지 마라. `확인됨 / 실패 / 미검증`을 분리하라.

## 1. 작업 트리와 정적 검사

먼저 다음을 기록한다.

```bash
git status --short --branch
git diff --check
```

변경의 중심은 다음 파일이다.

- `src-tauri/src/clamshell.rs`: off → 일회성 → 지속 → off, 원래 SleepDisabled 복원, 세대별 root 감시자
- `src-tauri/src/update.rs`: 앱 번들 버전 확인, `renamex_np(RENAME_SWAP)` 원자 교체, 수동 업데이트 재실행. Windows는 종료 뒤 helper 교체지만 Mac에는 적용되지 않는다.
- `src-tauri/src/login.rs`, `accounts.rs`, `usage.rs`, `lib.rs`, `src/main.ts`: 로그인 세션 ID, 재로그인, 삭제·갱신·캐시 경합 수정

코드를 읽고 macOS `cfg` 경로와 FFI 선언이 실제 Mac에서 컴파일되는지 확인한다. 문제가 확실하면 해당 범위만 최소 수정하고 아래 검사를 다시 한다.

```bash
cd src-tauri
cargo test
cargo check
cd ..
npm run tauri build -- --bundles app
```

빌드 결과는 `src-tauri/target/release/bundle/macos/switcher.app`이어야 한다. 테스트의 `mac_zip_atomically_swaps_bundle_and_keeps_rollback_copy`가 실제 macOS 파일 시스템에서 통과했는지 따로 적는다.

## 2. 클램셸 시작 상태 보존

앱을 조작하기 전에 다음 결과를 기록한다. `SleepDisabled` 줄이 없으면 **없음**으로 기록하고 임의로 0이라 적지 마라.

```bash
/usr/bin/pmset -g
/usr/bin/pmset -g custom
```

`~/.switcher/clamshell.state`가 이미 있으면 내용은 `mode`, `saved`, `revision`, `watcher` 네 줄만 확인할 수 있다. 그 외 계정·토큰 파일은 열지 마라. 기존 클램셸 상태가 켜져 있으면 먼저 사용자에게 알리고, 바로 덮어쓰지 마라.

감시자 확인은 다음처럼 명령줄 표식과 PID 개수만 본다.

```bash
# 주의: 감시자 본문 argv에 /usr/bin/grep이 들어 있어 `grep -v grep`으로 거르면
# 감시자 자신까지 걸러진다 (실측). ppid=1(launchd 입양)로 본체만 센다 —
# read_state 순간의 fork 자식은 부모 argv를 그대로 보이므로 ppid 필터가 필요하다.
/bin/ps -axo pid=,ppid=,uid=,command= | /usr/bin/awk '/SWITCHER_CLAMSHELL_WATCH=1/ && !/awk/ && $2==1 {print $1, $3}'
```

## 3. 버튼 상태 전환 실기기 검증

각 단계에서 UI 모드, `pmset -g`, `clamshell.state`, `clamshell.pid`, root 감시자 수를 기록한다.

1. off에서 한 번 클릭한다.
   - UI와 상태 파일은 mode 1이어야 한다.
   - SleepDisabled는 1이어야 한다.
   - 정확히 한 감시자가 살아 있어야 한다.
2. 사용자가 직접 덮개를 한 번 닫았다 다시 열도록 요청한다. 명령으로 가짜 수면을 만들지 마라.
   - 열린 뒤 원래 SleepDisabled 값으로 돌아와야 한다.
   - UI는 off로 돌아오고 상태·PID 파일과 감시자가 정리돼야 한다.
3. 다시 한 번 클릭한 뒤, 덮개를 닫기 전에 두 번째로 클릭한다.
   - mode 1 → mode 2가 되어야 한다.
   - 감시자는 중복 생성되지 않고 한 개여야 한다.
4. 앱을 정상 종료했다 다시 실행한다.
   - mode 2와 SleepDisabled 1이 유지돼야 한다.
   - 재실행 뒤에도 감시자는 한 개여야 한다.
5. mode 2에서 버튼을 한 번 더 눌러 off로 만든다.
   - 최초 시작 전에 기록한 SleepDisabled의 기능상 값으로 돌아와야 한다.
   - 상태·PID 파일과 감시자가 남지 않아야 한다.

시작 값이 1이었던 Mac에서는 off 후에도 1이 정상 복원값이다. 이를 실패로 오판하지 마라. 시작 시 줄이 없었던 경우에는 기능상 0 복원을 확인하고, 줄 자체가 새로 생겼는지도 별도로 기록한다.

## 4. 실패·경합·복구 검증

- off에서 켜기 버튼을 누른 뒤 관리자 승인 창을 취소한다. SleepDisabled와 파일·감시자가 시작 전 상태 그대로인지 확인한다.
- 버튼을 빠르게 두 번 눌러도 작업이 직렬화되고, 감시자 중복이나 손상된 상태 파일이 생기지 않는지 확인한다. 실제로 접수된 클릭 수에 따라 최종 mode가 1 또는 2일 수 있으므로 이벤트와 결과를 함께 적는다.
- mode 1과 mode 2 각각에서 앱만 강제 종료해 본다.
  - mode 1: 감시자는 남아 덮개 1회 사이클 뒤 원래 설정을 복원해야 한다.
  - mode 2: 앱 종료로 꺼지면 안 되며, 앱 재실행 시 같은 지속 상태로 붙어야 한다.
- 감시자 종료 경계와 mode 1 → 2 전환을 겹쳐도 낡은 감시자가 새 상태를 지우거나 원래 설정으로 되돌리지 않는지 확인한다.
- 재부팅 검증은 사용자가 허락할 때만 한다. 허락받았다면 **mode 2도 재부팅 후에는 원래 SleepDisabled로 복원되고 off로 시작하는지** 확인한다 (재무장 없음 — 사용자 결정 2026-08-12. 앱 재시작 생존은 살아 있는 감시자 입양으로만).

어떤 실패가 나도 먼저 `clamshell.state`의 `saved`와 시작 전 기록을 대조하라. UI로 off 복원이 불가능할 때만 사용자 허락을 받은 뒤 `sudo /usr/bin/pmset -a disablesleep <원래값>`으로 복구한다. 광범위한 전원 설정 초기화나 preference 삭제는 하지 마라.

## 5. 업데이트 검증

- `cargo test`의 macOS 임시 번들 교체 테스트로 다음을 확인한다.
  - zip 안 앱 버전과 기대 버전이 일치해야만 적용된다.
  - 교체 후 원래 경로에는 새 앱이 있고 `.app.old`에는 이전 앱이 남는다.
- 교체 중 어느 순간에도 원래 앱 경로가 비는 두 번 rename 방식이 사용되지 않는다.
- 실제로 더 높은 공식 릴리스가 이미 존재하는 경우에만 설치된 이전 버전에서 트레이의 **업데이트 확인**을 눌러 다운로드 → 교체 → 자동 재실행 → 새 버전 표시까지 확인한다.
- 검증을 위해 새 릴리스나 태그를 만들지 마라. 더 높은 공식 릴리스가 없으면 실제 네트워크 업데이트 E2E는 **미검증**으로 남겨라.
- 자동 확인이 먼저 업데이트를 받아 둔 경우, 이후 수동 버튼이 네트워크 재요청 없이 즉시 재실행 경로로 이어지는지도 가능한 환경에서 확인한다.

## 6. 최종 보고 형식

다음만 간결하게 보고한다.

1. 현재 커밋/작업 트리와 실행한 명령
2. `cargo test`, `cargo check`, `.app` 빌드 결과
3. 클램셸 시나리오별 `확인됨 / 실패 / 미검증`
4. 업데이트 단위 테스트와 실제 업데이트 E2E를 각각 분리한 결과
5. 발견한 결함, 수정한 파일, 남은 위험
6. 최종 SleepDisabled가 최초 값으로 복구됐는지와 남은 감시자·상태 파일 수

개인정보나 자격증명 내용은 보고에 포함하지 마라. 스크린샷이 꼭 필요하면 계정 영역을 완전히 가리고, `pmset`·클램셸 버튼·상태 숫자만 보이게 잘라서 촬영하라.
