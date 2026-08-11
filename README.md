<h1 align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/logo.svg" width="30" alt="" />
  switcher
</h1>

<p align="center">
  <strong>Claude Code · Codex CLI 다중 계정 전환 위젯</strong><br />
  다시 로그인하지 않고 계정을 바꾸고, 한도와 리셋 시간을 한눈에 확인하세요.
</p>

<p align="center">
  <a href="https://github.com/Youkamii/switcher/releases/latest"><img src="https://img.shields.io/github/v/release/Youkamii/switcher?style=flat-square&label=release" alt="latest release" /></a>
  <a href="https://www.npmjs.com/package/switcher-widget"><img src="https://img.shields.io/npm/v/switcher-widget?style=flat-square" alt="npm version" /></a>
  <img src="https://img.shields.io/badge/Windows-10%2F11-0078D4?style=flat-square&logo=windows" alt="Windows 10/11" />
  <img src="https://img.shields.io/badge/macOS-Apple%20Silicon-000000?style=flat-square&logo=apple" alt="macOS Apple Silicon" />
</p>

<p align="center">
  <strong>한국어</strong> ·
  <a href="docs/README.en.md">English</a> ·
  <a href="docs/README.ja.md">日本語</a> ·
  <a href="docs/README.zh-CN.md">简体中文</a> ·
  <a href="docs/README.zh-TW.md">繁體中文</a> ·
  <a href="docs/README.hi.md">हिन्दी</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/screenshot.png" alt="switcher Type 1, Type 2, Type 3" />
</p>

## 한 번 로그인하고, 위젯에서 바꾸세요

Claude Code와 Codex CLI는 한 번에 한 계정만 활성화합니다. 계정을 여러 개 쓰면 한도가 찰 때마다 로그아웃하고, 브라우저 인증을 다시 하고, 어느 계정에 여유가 남았는지 따로 확인해야 합니다.

switcher는 계정별 로그인을 한 번만 받아 안전하게 보관합니다. 이후에는 위젯에서 계정을 바꾸고, 각 계정의 **5시간·주간·모델별 사용량과 리셋 시간**을 바로 확인할 수 있습니다.

- Claude Code · Codex CLI · GitHub CLI 계정 전환
- 계정별 사용량, 리셋 시간, 구독 등급 표시
- 전체·컴팩트·미니멀 3가지 보기 모드
- 한도 임박 시 자동 전환하는 TFSD 자율주행
- 시스템 모니터, 메모장, 화면 밝기, 블랙 모니터
- Windows · macOS 지원, 6개 UI 언어

## 설치

### npm으로 설치 — 권장

Node.js 18 이상이 필요합니다. 처음 실행할 때 운영체제에 맞는 최신 빌드를 자동으로 받습니다.

```sh
npm install -g switcher-widget
switcher
```

브라우저에서 실행 파일을 직접 내려받는 방식이 아니므로 Windows SmartScreen이나 macOS의 다운로드 경고를 피할 수 있습니다.

### 직접 다운로드

[최신 릴리스](https://github.com/Youkamii/switcher/releases/latest)에서 운영체제에 맞는 파일을 받으세요.

| 운영체제 | 파일 | 참고 |
|---|---|---|
| Windows 10/11 64비트 | `switcher-win-x64.zip` | 압축을 풀고 `switcher.exe` 실행 |
| macOS Apple Silicon | `switcher-mac-arm64.zip` | 압축을 풀고 `switcher.app` 실행 |

직접 다운로드한 빌드는 코드 서명이 없습니다. Windows에서 SmartScreen이 뜨면 **추가 정보 → 실행**, macOS에서 차단되면 **시스템 설정 → 개인정보 보호 및 보안 → 그래도 열기**를 사용하세요.

## 핵심 기능

### 1. 계정 전환과 사용량 확인

활성 계정은 밝게 표시됩니다. 다른 계정의 **이 계정으로 전환** 버튼을 누르면 새로 여는 터미널부터 해당 계정이 적용됩니다.

Type 2와 Type 3에서는 계정 카드를 **더블클릭**해 전환합니다. 카드 밖의 빈 영역은 클릭과 드래그가 뒤 창으로 통과하므로, 위젯을 화면에 계속 띄워 두어도 작업을 방해하지 않습니다.

사용량 막대에는 사용률 숫자가 함께 표시되고, 오른쪽에는 실제 리셋까지 남은 시간이 표시됩니다.

Type 2의 리셋 시간은 콜론 표기로 압축됩니다. 24시간 미만은 `시:분`(예: `2:21`), 그 이상은 `일::시`(예: `5::17`)로, 콜론이 두 개면 일 단위라는 뜻입니다. 위젯 모드는 클릭이 뒤 창으로 통과해 툴팁을 띄울 수 없어 여기에 적어 둡니다.

### 2. 화면에 맞춰 바꾸는 세 가지 모드

오른쪽 위 Type 버튼으로 모드를 순환합니다.

| 모드 | 용도 |
|---|---|
| **Type 1** | 계정 추가·삭제, GitHub, DISPLAY, SYSTEM까지 모두 조작하는 전체 화면 |
| **Type 2** | 이메일과 구독 정보를 유지한 컴팩트 위젯 |
| **Type 3** | 폭 120px의 미니멀 위젯. 라벨·사용률 막대만 표시 |

Type 1에서는 섹션 제목을 끌어 CLAUDE·CODEX·GITHUB·DISPLAY·SYSTEM 순서를 바꿀 수 있습니다. ☰ 핸들로 위젯을 이동합니다.

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/size-compare.png" width="620" alt="Codex 데스크톱 앱의 최소 크기 펫 옆에 놓은 switcher Type 3" />
  <br />
  <sub>Type 3와 Codex 데스크톱 앱 펫의 최소 크기 비교</sub>
</p>

### 3. TFSD 자율주행

TFSD(Token Full Self-Driving)를 켜면 활성 계정의 사용량 창 중 하나가 90%에 도달했을 때, 모든 창에 여유가 있는 계정 중 가장 넉넉한 곳으로 자동 전환합니다.

- 타이틀바의 🚗 버튼 또는 트레이 설정에서 켜기
- 자율주행 중인 활성 카드에는 **T 워터마크** 표시
- 꽉 찬 창이 30분 안에 리셋되면 계정을 바꾸지 않고 대기
- 수동으로 계정을 바꾸면 자율주행 자동 해제
- 전환 기록은 `~/.switcher/tfsd-history.log`에 저장

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/tfsd.png" width="340" alt="TFSD가 켜져 활성 카드에 T 워터마크가 표시된 화면" />
</p>

### 4. 투명도와 클릭 통과

투명도 슬라이더를 내리면 배경부터 사라지고, 글자와 막대가 차례로 옅어집니다. 최하에서는 사용량 그래프만 남아 뒤 창 위에 자연스럽게 겹쳐집니다.

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/transparency.gif" width="400" alt="코드 편집기 배경 위에서 투명도를 조절하는 switcher Type 3" />
</p>

### 5. 블랙 모니터

🌙 버튼을 누르면 모든 모니터를 검은 오버레이로 덮습니다. 커서를 움직이면 주변만 연기처럼 걷혀 뒤 화면이 보이고, 다시 검게 차오릅니다.

- 마우스를 1~2초 세게 흔들거나 `Esc`로 해제
- 해제할 때 마지막 커서 위치에서 빛이 퍼지는 연출
- Windows에서는 밝기 연동, macOS에서는 안전을 위해 오버레이만 사용
- macOS에서는 전체 화면 앱 위를 덮지 못함

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/black.gif" width="648" alt="코드 편집기 화면을 덮은 블랙 모니터에서 커서 주변만 보이고 다시 해제되는 모습" />
</p>

### 6. 시스템 모니터

📊 버튼을 누르면 위젯 안에 SYSTEM 섹션이 붙습니다.

- CPU 사용률과 60초 변화 그래프
- 메모리 사용량
- 디스크 읽기·쓰기. Windows에서는 실제 디스크 활성 시간으로 막대 표시
- 네트워크 다운로드·업로드

표시는 빠르게 읽을 수 있도록 단위 없는 정수로 정리되어 있습니다. CPU는 %, 메모리는 GB, 디스크와 네트워크는 MB/s 기준입니다.

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/system.png" width="440" alt="CPU, 메모리, 디스크, 네트워크를 표시하는 SYSTEM 섹션" />
</p>

### 7. 메모장

Type 2·3의 📝 버튼을 누르면 탭 5개짜리 메모창이 열립니다. 입력 내용은 자동 저장되고, 메모창 투명도는 위젯과 따로 조절할 수 있습니다.

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/memo.png" width="300" alt="탭 5개와 투명도 조절이 있는 switcher 메모장" />
</p>

### 그 밖의 기능

| 기능 | 설명 |
|---|---|
| ☕ 클램셸 슬립 방지 | macOS에서 덮개를 닫아도 잠들지 않아 터미널 작업이 계속 돌아갑니다. 한 번 누르면 이번 한 번만(덮개를 닫았다 열면 자동 복원), 두 번 누르면 계속. 켤 때 관리자 암호 1회 |
| 🙈 계정 정보 가리기 | 이메일과 GitHub 계정명을 흐리게 처리해 화면 공유·스크린샷 노출 방지 |
| DISPLAY | 모니터별 실제 백라이트 조절. Windows는 DDC/CI, macOS는 내장 디스플레이 지원 |
| GitHub 계정 전환 | `gh`에 로그인된 계정을 전환하고 HTTPS `git push/pull`에 반영 |
| 자동 업데이트 | 실행 시 새 버전을 받아 다음 실행부터 적용. 트레이의 **업데이트 확인**은 적용 후 자동 재시작 |
| 자동 실행 | 부팅 시 자동 실행. 트레이 설정에서 끄기 가능 |
| Windows 바로가기 | 첫 실행 때 바탕 화면에 한 번 생성. 사용자가 지우면 다시 만들지 않음 |
| macOS 오버뷰 | 모든 Space와 전체 화면 앱 위에서 위젯 표시 |
| 다국어 UI | 한국어·영어·일본어·중국어 간체·중국어 번체·힌디 |

## 처음 설정하기

### 계정 추가

1. Type 1에서 Claude 또는 Codex의 **+ 계정 추가**를 누릅니다.
2. 위젯에 표시된 주소를 브라우저에서 엽니다.
3. Claude는 브라우저에 표시된 코드를 위젯 입력칸에 붙여넣습니다.
4. Codex는 브라우저에서 15분 유효 일회용 코드를 입력합니다.
5. 로그인이 끝나면 계정이 카드로 추가됩니다. 현재 활성 계정은 바뀌지 않습니다.

Codex 장치 코드 인증이 꺼져 있으면 로그인할 수 없습니다. 개인 계정은 ChatGPT **설정 → 보안 → Codex 장치 코드 인증**, 팀·비즈니스 계정은 관리자의 워크스페이스 권한 설정에서 켜세요.

### GitHub 계정 추가

[GitHub CLI](https://cli.github.com)가 설치되어 있으면 GITHUB 섹션이 나타납니다. **+ 계정 추가**를 누르고 브라우저에서 장치 코드를 승인하세요.

GitHub 전환은 HTTPS 리모트에만 적용됩니다. SSH 리모트는 SSH 키가 계정을 결정하며, `git config user.name/email`이나 VS Code·Copilot 로그인은 바뀌지 않습니다.

## 데이터와 동작 방식

switcher는 CLI가 실제로 사용하는 인증 저장소를 읽고, 계정별 복사본을 `~/.switcher/profiles/`에 보관합니다.

| 대상 | 활성 인증 저장소 |
|---|---|
| Claude Code · Windows | `~/.claude/.credentials.json` |
| Claude Code · macOS | 키체인의 `Claude Code-credentials` |
| Codex CLI · Windows/macOS | `~/.codex/auth.json` |

계정 전환은 반드시 다음 순서로 진행됩니다.

1. 현재 활성 인증을 현재 계정 프로필에 먼저 백업
2. 선택한 계정 프로필을 활성 위치로 복사

이 순서를 지켜야 CLI가 자동 갱신한 최신 토큰을 잃지 않습니다. 토큰 값은 로그나 오류 메시지에 출력하지 않습니다.

대화 기록, 메모리, 프로젝트 설정은 인증 파일과 별개이므로 계정을 바꿔도 그대로 유지됩니다. 다만 실행 중인 Claude Code·Codex 세션은 시작할 때 읽은 인증을 계속 사용할 수 있으므로, 계정 전환 후에는 새 터미널 세션을 여는 것이 안전합니다.

## 트레이와 업데이트

- Windows는 작업 표시줄 알림 영역, macOS는 메뉴 막대에 W 아이콘으로 상주합니다.
- 아이콘 좌클릭으로 위젯을 열고 숨깁니다.
- 창을 닫아도 앱은 종료되지 않습니다. 완전히 끄려면 트레이 메뉴의 **종료**를 사용하세요.
- 언어, 자동 업데이트, 자동 실행, TFSD, 표시할 섹션을 트레이 설정에서 바꿀 수 있습니다.
- 수동 **업데이트 확인**은 새 버전을 내려받고 위젯을 자동 재시작합니다.

## 문제 해결

<details>
<summary><strong>Codex 계정 추가가 승인 단계에서 거부됩니다</strong></summary>

ChatGPT 계정에서 장치 코드 인증을 켜야 합니다. 개인 계정은 **설정 → 보안**, 팀 계정은 관리자의 워크스페이스 권한 설정을 확인하세요.
</details>

<details>
<summary><strong>Windows에서 화면 밝기가 바뀌지 않습니다</strong></summary>

모니터 OSD 설정에서 DDC/CI를 켜세요. 일부 모니터와 연결 방식은 DDC/CI를 지원하지 않습니다.
</details>

<details>
<summary><strong>macOS에서 외장 모니터 밝기를 조절할 수 없습니다</strong></summary>

현재 macOS 빌드는 내장 디스플레이 밝기만 지원합니다. 외장 모니터에는 미지원 안내가 표시됩니다.
</details>

<details>
<summary><strong>계정을 전환했는데 이미 열려 있던 CLI가 그대로입니다</strong></summary>

기존 CLI 세션이 시작 당시 인증을 들고 있을 수 있습니다. 새 터미널에서 Claude Code 또는 Codex를 다시 시작하세요.
</details>

<details>
<summary><strong>프로필을 삭제했는데 계정을 전환하니 다시 생깁니다</strong></summary>

의도된 동작입니다. 삭제는 보관함 사본만 지우고 로그인 자체는 남는데, 계정을 전환할 때마다 현재 로그인 계정이 자동 백업되므로 활성 계정의 프로필은 다음 전환 때 되살아납니다. 완전히 정리하려면 먼저 다른 계정으로 전환한 뒤 삭제하세요.
</details>

## 직접 빌드

[Node.js](https://nodejs.org) 18 이상과 [Rust](https://rustup.rs)가 필요합니다.

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm install
npm run tauri build -- --no-bundle
```

| 작업 | 명령 |
|---|---|
| 개발 실행 | `npm run tauri dev` |
| Rust 빠른 검사 | `cd src-tauri && cargo check` |
| Rust 테스트 | `cd src-tauri && cargo test` |
| Windows 포터블 빌드 | `npm run tauri build -- --no-bundle` |
| macOS 앱 빌드 | `npm run tauri build -- --bundles app` |

결과물:

- Windows: `src-tauri/target/release/switcher.exe`
- macOS: `src-tauri/target/release/bundle/macos/switcher.app`

## 기술 구성

- Tauri 2 + Rust
- Vanilla TypeScript + Vite
- Windows WebView2 / macOS WKWebView
- PTY 기반 Claude 로그인, 장치 코드 기반 Codex 로그인
- 인증 전환과 사용량 조회는 Rust에서 처리

---

<p align="center">
  MIT License · 개인·상업용 모두 사용 가능
</p>
