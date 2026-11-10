# 개선 적용 현황

작업 브랜치: `feat/connection-workspace` · 2026-11-10. UI·API 기능과 인증 오류 처리를 개선했다. CLI 실행 여부만으로 갱신을 막던 제한은 제거했다. macOS 앱 빌드·서명 검증을 마치고, 실행 중이던 앱을 새 빌드 경로의 실행 파일로 교체했다. 로컬 커밋이며 푸시·배포는 하지 않았다.

[전체 화면 한 장](design/complete-preview.png) · [원본 크기 보기](design/complete-preview.html) · [API 등록·사용법](API_CONNECTIONS.md)

## 적용한 동작

| 요청 | 적용 내용 |
| --- | --- |
| 버튼 모양·나열 | 계정 카드 우측에 전환/사용 중 슬롯과 SVG 아이콘 3개(다시 연결·이름·삭제)를 배치. 활성·비활성 계정의 위치가 같다. 호버·삭제 확인 때 다음 카드가 움직이지 않는다. |
| 다른 도구가 안 보임 | 메모·자동 전환·가리기·잠자기 방지·화면 끄기를 기본 펼침으로 표시. 같은 선 두께의 아이콘을 사용하며 일반 모드에는 이름도 표시. |
| 네 모드 유지 | 일반 420px·컴팩트 280px·미니멀 120px·엣지 80px 유지. 일반 화면에서 네 모드를 직접 선택할 수 있다. |
| AI 미사용 빈자리 | 숨긴 제공자 행·구분선의 공간 제거. 엣지 손잡이 높이도 실제 표시 행에 맞춰 계산. AI를 모두 숨기면 SYSTEM과 도구만 남는다. |
| 잦은 인증 오류 | CLI를 실행한 상태에서도 다른 계정의 사용량·토큰 갱신이 진행된다. 활성 계정의 토큰은 CLI가 관리하고 위젯은 중복 회전하지 않는다. 액세스 토큰 갱신 필요, 갱신 토큰 거부, 권한 오류, 요청 제한을 구분. 마지막 사용량을 보여줄 때도 실패 이유를 남긴다. 카드의 다시 연결에서 기존 계정을 재인증할 수 있다. |
| API 등록과 사용 | 제공자별 **계정 추가**에서 **구독 로그인 / API 키** 선택. API는 **저장하고 사용**으로 일반 CLI에 적용. 별도 API 등록 버튼 제거. 기존 연결은 **사용/해제**, 확인·교체·삭제. 적용 실패 시 저장한 키로 재시도하며 네 모드에 선택된 API를 표시한다. |
| API 사용량·예산 | 각 연결 기본 월 $195, 등록/사용량 설정에서 변경. 직접 금액 입력 또는 관리자 키 + API key ID로 자동 비용 조회. 네 모드에 금액·막대, 초과 금액 표시. UTC 월 변경·권한 오류·부분 조회 실패 처리. |
| API 키 보호 | 저장 연결은 macOS Keychain/Windows DPAPI. Claude는 보안 저장소 helper, Codex는 표준 활성 auth.json(Unix 0600)을 사용한다. OAuth·설정을 먼저 백업하고 해제·구독 전환·삭제 시 복원한다. 키 입력·목록·로그 보호 유지. |
| 상태·오류 복구 | 확인 상태와 시각을 저장. 키 교체 시 초기화, 확인 중 교체·삭제 시 낡은 결과 폐기. 동일 이름의 암묵적 교체 차단. 보안 저장 실패 시 메타데이터 복구. |
| 등록 중 창 이동 | 등록·교체·완료 화면의 제목과 원래 타이틀바 영역에서 네이티브 창 드래그. 입력값 유지, 입력칸·버튼·우클릭 제외. |
| 비활성 macOS 창 | API 창 닫기와 키 입력 정리를 DOM close 이벤트·타이머에만 맡기지 않고 즉시 처리. 삭제 확인은 실제 경과 시간과 사용자 동작으로 해제. |

구독 토큰 수명을 바꾸거나 만료 자체를 없앤 것은 아니다. 활성 파일 먼저 백업 → 대상 프로필 복사 순서, 활성 계정 직접 갱신 보호, 프로필별 갱신·전환 잠금과 사이드카 복구는 유지했다. **Claude/Codex를 실행 중이라는 이유로 다른 계정의 갱신을 막지 않는다.** `f28add7`에서 도입했던 제공자 전체 차단·종료 감시·“CLI 사용 중” 안내는 잘못된 제약이라 제거했다. API는 앱에서 명시적으로 선택하며 다음 일반 CLI 실행부터 적용된다. API 선택 시 TFSD를 해제하고, 구독으로 돌아가면 앱이 적용한 설정만 복원한다. 자동 과금 대체는 없다.

## 검증 결과

| 검사 | 결과 |
| --- | --- |
| 프론트 단위 검사 `npm test` | 93 통과 |
| Rust `cargo test --lib` | 239 통과 / 18 ignored |
| CLI 사용 중 갱신 회귀 검사 | Claude/Codex 이름의 무해한 프로세스를 실행한 상태에서 다른 계정은 1회 갱신 성공·재사용, 활성 계정은 요청 0건·토큰 무변경. 두 제공자 모두 로컬 서버로 검사 |
| API 집중 검사 `cargo test api_connections::tests -- --include-ignored` | 11 통과. 실제 macOS 키체인 가짜 키 왕복·삭제, 설치된 Claude/Codex `--version` 실행 포함 |
| API 선택 집중 검사 `cargo test api_connections::activation::tests -- --include-ignored` | 7 통과. 설정·OAuth 백업/복원, 외부 변경 보존, 쓰기 실패 복구. 설치된 Claude/Codex의 실제 인증 상태 명령으로 API 인식 확인(가짜 키·격리 홈), 활성 연결 삭제 복원 |
| API 비용 집중 검사 | 6 통과. 연결별 $195 기본값, UTC 월 변경, 0/미조회 구분, 초과/잘못된 금액, 관리자 키 분리·저장 실패 복구, HTTP 단위·페이지 합산·ID 필터·실패·응답 경합 |
| Chromium 실제 번들 검사 | 123 통과. 통합 계정 추가·네 모드·AI 숨김·API 전환/복원·사용량 입력/자동조회/초과/오류·드래그 호출·입력 보존 검사 |
| macOS WebKit 실제 번들 검사 | 93 통과. 비활성 NSPanel에 WKWebView를 붙여 같은 기능·배치 검사 수행 |
| 기존 숨김·배치 회귀 검사 | 8 통과. 최초 기준선의 4건 실패 해소 |
| TypeScript/Vite | 빌드 성공 |
| macOS Tauri 앱 | 빌드 성공. ad-hoc 서명 검증 성공, notarization 없음 |

[Chromium 검사 원문](four-mode-test-results.json) · [WebKit 검사 원문](webkit-test-results.json) · [최초 리뷰·기준선](REVIEW_TEST_RESULTS.md)

UI 검사는 **실제 배포 프론트 + 가짜 Tauri 응답**을 사용한다. WebKit은 임시·비영속 데이터 저장소와 화면 밖 NSPanel에서 실행하며, 비활성 페이지의 타이머 지연 때문에 테스트 동작 간 진행은 네이티브 테스트 호스트가 구동한다. 실제 사용자 프로필·키·기존 브라우저 세션을 사용하지 않는다. 이미지의 계정·사용량·연결 확인 결과도 검증용 예시다. 운영 앱은 새 빌드 실행 파일 경로와 새 프로세스를 확인했다. 최종 실행은 사용자가 선택한 엣지 모드(80px)이며, 앞선 실행에서 일반 모드(420px)도 확인했다. 운영 창의 접근성 트리와 캡처가 정상 반환되지 않아 네이티브 포인터로 API 등록을 끝냈다고 기록하지 않는다. 버튼 동작은 별도 WebKit 검증 호스트에서 검사했다. 등록창의 드래그는 Tauri 네이티브 호출 전달과 이동 이벤트 이후 입력 보존까지 확인했으며, 운영 창을 실제 포인터로 끌어 이동한 검증과 구분한다. 사용자 요청으로 기존 저장 API를 실제 적용한 뒤 Claude 인증 상태 명령에서 loggedIn=true, authMethod=api_key_helper, apiKeySource=apiKeyHelper를 확인했다. 키 원문·계정 정보는 출력하지 않았다.

API HTTP 검사는 로컬 서버에서 200(정상/잘못된 본문)·401·403·429·503·리다이렉트, 인증 헤더, 안전한 오류 표시를 확인했다. 실제 키로 생성 요청이나 유료 호출은 하지 않았다. 키체인에 만든 가짜 테스트 항목은 삭제했다. 실행 중인 Switcher는 정상 종료 후 새 빌드로 다시 실행했다. 사용자 Claude/Codex 세션은 종료하지 않았다. 여러 Switcher 백업 앱이 같은 bundle ID로 등록되어 있어 이름만으로 실행하면 구버전이 열리는 것을 확인했고, 최종 실행 파일은 `src-tauri/target/release/bundle/macos/switcher.app/Contents/MacOS/switcher` 경로로 확인했다.

재현 명령:

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml api_connections::tests -- --include-ignored
cargo test --manifest-path src-tauri/Cargo.toml api_connections::activation::tests -- --include-ignored
cargo test --manifest-path src-tauri/Cargo.toml api_connections::costs::tests
node scripts/review-layout.mjs
node scripts/test-workspace-ui.mjs --all-modes docs/design --output docs/four-mode-test-results.json
node scripts/test-workspace-ui.mjs --webkit --output docs/webkit-test-results.json
npm run tauri build -- --bundles app
```

`--include-ignored`는 위 API 모듈에만 한정한다. 해당 로컬 검사는 macOS 키체인에 고유한 가짜 항목을 만들었다 지우고, 설치된 CLI의 버전 또는 인증 상태 명령을 실행한다. 전체 ignored 실계정 검사를 실행하는 명령이 아니다. WebKit 검사는 macOS와 Xcode Command Line Tools가 필요하다.

## 이슈와 남은 범위

| 이슈 | 상태·남은 범위 |
| --- | --- |
| [#179 UI](https://github.com/Youkamii/switcher/issues/179) | 요청한 배치·아이콘·4개 모드·숨김 수정 적용. Windows 실화면과 운영 앱의 실제 포인터 상호작용 검증은 남음. |
| [#180 인증](https://github.com/Youkamii/switcher/issues/180) | 오류 구분·재연결·Codex 경로/저장소 보호 적용. CLI 실행 여부로 전체 갱신을 막던 제한 제거. 외부 CLI와 모든 토큰 회전을 원자적으로 조정하는 기능, Codex 네이티브 키링 어댑터는 지원 범위 밖. |
| [#181 API](https://github.com/Youkamii/switcher/issues/181) | 통합 계정 추가, 일반 CLI API 적용·구독 복원, API별 $195 예산·비용 표시 적용. 실제 관리자 비용 조회·모델 생성과 Windows 실행은 미검증. 새 UI 문구는 한국어·영어 제공. |
| [#182 업데이트 서명](https://github.com/Youkamii/switcher/issues/182) | 리뷰에서 확인한 별도 후속 작업. 릴리스 신뢰키·서명 배포 체계는 이번 UI/API 변경에 포함하지 않음. |

Windows 전체 크로스 컴파일은 이 Mac에 Windows SDK 헤더가 없어 `ring` 단계에서 중단됐다. DPAPI 함수의 Windows 타깃 타입 검사는 별도 최소 크레이트에서 통과했지만 Windows 실행 검증을 대신하지 않는다. 기존 Rust 경고 4개는 남아 있다. 실제 유료 모델 생성은 수행하지 않았다.

## 커밋

- `9963d46` 전체 리뷰·개선 계획·초기 재현 검사
- `b18c7c0` 숨긴 AI 공간·호버 이동 수정
- `b0c0b51` 인증 오류 분류·실패 이유 보존
- `9f21ce4` Codex 경로·인증 저장소 보호
- `672474e` API 보안 저장소·실행 명령 기반
- `1c6c5db` 네 모드 도구 접근성 복원
- `b2ef2c1` API 확인 상태 영속화·명시적 교체·오류 복구·CLI 통합 검사
- `75161f2` 계정 아이콘 배치·API 등록부터 실행까지 연결·Chromium/WebKit 검사
- `f28add7`의 제공자 전체 갱신 차단은 철회. CLI를 사용하면서 다른 계정도 갱신할 수 있어야 한다는 기준으로 회귀 검사를 교체했다.

추가 적용: `fd2b233` — 수동 실행 명령 복사를 일반 CLI API 선택·구독 복원으로 교체하고 API 등록창 드래그 추가. 이후 통합 계정 추가와 API별 사용량·예산을 반영했다.

- `8cdc624` — 구독/API 통합 계정 추가, API별 월 $195 예산·비용 조회/직접 입력, 미조회·초과·월 변경 처리와 회귀 검사.
