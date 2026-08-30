# macOS 자격증명 전환·Vault 경계 검증 보고서

- 날짜: 2026-08-31
- 대상: `feat/macos-credential-vault`
- 환경: Apple Silicon macOS, Tauri `.app` arm64 빌드
- 상태: `DONE_WITH_CONCERNS`

## 증상과 확인된 원인

1. Claude CLI 버전에 따라 macOS Keychain 값이 raw JSON 또는 hex 문자열로 남는데, 격리 로그인과 기존 프로필 적용 경로가 이를 일관되게 정규화하지 않았다.
2. Keychain과 legacy credential 파일 사이에서 일부 쓰기 또는 조건부 원복이 실패하면 혼합 상태가 다음 저장·사용량 조회·Vault 내보내기에 정상 상태처럼 사용될 수 있었다.
3. 복구 표식이 있어도 fresh usage cache가 먼저 반환되고, TFSD가 `oauthAccount`만으로 활성 계정을 판정할 수 있었다.
4. 복구 전환 직전 pending refresh sidecar가 활성 credential을 읽으려 해, 정작 복구 표식 때문에 전환 자체가 막힐 수 있었다.
5. 활성 계정의 격리 재로그인은 최초 활성 판정 뒤 외부 CLI 로그인을 다시 확인하지 않아 새 외부 로그인을 덮을 수 있었다. 또한 복구 표식과 stale `oauthAccount`만 남고 live credential이 모두 없으면 fresh 로그인 결과를 저장하기 전에 실패했다.
6. active relogin의 fresh 프로필 저장 뒤 root recovery marker 생성이 일시 실패하면, 다음 정상 전환의 활성 백업이 유일한 fresh 사본을 old live credential로 덮을 수 있었다.
7. Vault가 내보낼 provider가 아닌 다른 provider의 깨진 활성 상태까지 검사했고, 활성 신원을 식별할 수 없을 때 저장 프로필 credential을 활성 사본처럼 내보낼 여지가 있었다.
8. 계정 전환과 종료/업데이트 예약이 서로 다른 원자 상태를 사용해, 검사와 실제 blocking worker 사이에 종료가 끼어들 수 있었다.
9. atomic rename 뒤 상위 디렉터리를 동기화하지 않아 전원/OS crash에서 새 파일 또는 신규 프로필 디렉터리 엔트리의 내구성이 충분하지 않았다.
10. active relogin의 보호 sidecar가 `meta.json`보다 먼저 남으면 중단 프로필의 소유 계정이 화면과 검색에서 사라졌다. 같은 자동 이름의 다른 계정이 이를 차지해 fresh credential을 덮을 수 있었고, 프로필 디렉터리 심볼릭 링크는 전역 pending 검사에서 빠질 수 있었다.

## 적용한 수정

- raw/hex Claude credential을 저장·적용 관문에서 정규화하고 민감한 임시 문자열을 즉시 zeroize한다.
- macOS Keychain+legacy 전환에 영속 복구 저널을 추가했다. 준비와 최종 외부 변경 검사가 끝난 뒤 첫 live write 직전에 표식을 세우고, 완전한 성공 뒤에만 해제한다.
- 혼합 원복 충돌에서는 어느 저장소도 파괴하지 않고 표식을 유지한다. 표식이 있으면 일반 live read, save, usage, Vault export를 fail-closed로 막고 사용자가 고른 프로필로만 수렴시킨다.
- 복구 전환의 pending 응답은 대상 프로필에 먼저 병합하되 차단된 live repair는 건너뛴다. 선택한 최신 프로필이 곧 활성 위치에 적용된다.
- active usage fetch 앞뒤에 복구 관문을 두어 memory/disk fresh cache와 stale fallback도 차단한다. 목록은 프로필을 계속 보여주되 혼합 `oauthAccount`를 active 판정에 쓰지 않는다.
- 활성 Claude 재로그인은 소유권 `meta.json`을 먼저 영속화한 다음 `.claude-live-apply-pending` 보호 sidecar를 세운다. 중단된 옛 프로필은 meta가 없어도 저장된 `oauthAccount`로 소유권을 복원하며, 같은 계정만 protected repair를 이어갈 수 있다.
- 새 credential을 프로필에 저장한 뒤 전환과 같은 expected guard·조건부 원복·적용 후 안정성 검사를 공유한다. live credential이 없는 recovery도 당시의 `oauthAccount`를 guard에 보존해 OAuth-only 외부 변경을 거부한다.
- 보호 sidecar가 남아 있으면 일반 save/backup/usage/Vault/TFSD를 fail-closed로 막는다. 사용자가 그 프로필을 선택하거나 같은 계정으로 재로그인하면 fresh 사본을 활성 위치에 적용한 뒤에만 sidecar를 해제한다.
- 프로필 디렉터리 심볼릭 링크는 전역 복구 관문과 프로필 열거 양쪽에서 fail-closed로 거부한다.
- Vault 목록/내보내기는 활성 신원을 증명할 수 없으면 실패하고, 가져오기 payload에 실제로 포함된 provider만 검사한다.
- 계정 전환과 shutdown reservation을 하나의 mutex 상태로 직렬화하고, async 호출이 취소돼도 실제 blocking worker가 guard를 소유한다.
- atomic file rename 뒤 대상 디렉터리를 fsync하고, 새 프로필 디렉터리 체인은 새 디렉터리와 부모 엔트리까지 fsync한다.

## 회귀 검증

- `cargo test -q`: 265 passed, 0 failed, 15 ignored
- `cargo test -q -- --test-threads=1`: 265 passed, 0 failed, 15 ignored
- `npm test`: 75 passed, 0 failed
- `npm run tauri build -- --bundles app`: 성공
- `codesign --verify --deep --strict`: 성공
- 번들 실행 파일: Mach-O 64-bit arm64
- 번들 버전: 1.8.5 / 1.8.5
- `cargo clippy --all-targets`: 성공(기존 스타일 경고 포함, 오류 없음)
- `git diff --check`: 성공
- macOS `security` 임시 fixture: 같은 service의 서로 다른 account를 각각 읽기·갱신·삭제했고, 테스트 종료 뒤 `switcher-*` 시험 항목 잔재 없음
- 독립 적대 검증: orphan 소유권/동일 계정 복구/OAuth-only 변경/symlink 차단을 포함한 exact 회귀, marker/pending/cache/prewrite/relogin/Vault, lifecycle interlock 4개, Windows 최소 타입 검사 통과. 재현 가능한 High/Medium 결함 없음.

## 의도적으로 실행하지 않은 검증

- `real_` ignored 테스트 15개: 사용자의 실제 계정·실토큰·Keychain 상태를 바꿀 수 있어 실행하지 않았다.
- 전체 Windows 빌드: 현재 macOS 호스트에는 Windows MSVC 도구체인이 없다. Windows 조건부 코드의 최소 타입 검사는 독립 검토에서 통과했다.
- notarization: Apple 배포 자격증명이 없어 ad-hoc 서명까지만 검증했다.

## 남은 위험

1. `/usr/bin/security`는 “읽은 값이 그대로일 때만 쓰기” 형태의 값 기반 CAS를 제공하지 않는다. 최종 guard와 Keychain 쓰기 사이에 같은 macOS 사용자로 다른 writer가 정확히 끼어드는 극단적 경합은 완전히 제거할 수 없다.
2. 파일·디렉터리 `fsync`까지 적용했지만, macOS의 물리적 저장장치 flush를 강제하는 `F_FULLFSYNC` 수준의 갑작스러운 전원 차단 보장은 별도 fault-injection 검증이 필요하다.

두 항목은 현재 수정의 완료 주장을 흐리지 않도록 후속 견고성 이슈로 분리한다.

## 관련 이슈

- #124 macOS Keychain service+account 정확 조회
- #125 Claude 계정 전환 실패 원복
- #131 macOS Keychain 외부 writer 경합과 전원 차단 내구성 강화
