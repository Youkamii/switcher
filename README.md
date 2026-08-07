<h1><img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/logo.svg" width="26" alt="" /> switcher</h1>

**한국어** | [English](https://github.com/Youkamii/switcher/blob/main/docs/README.en.md) | [日本語](https://github.com/Youkamii/switcher/blob/main/docs/README.ja.md) | [简体中文](https://github.com/Youkamii/switcher/blob/main/docs/README.zh-CN.md) | [繁體中文](https://github.com/Youkamii/switcher/blob/main/docs/README.zh-TW.md) | [हिन्दी](https://github.com/Youkamii/switcher/blob/main/docs/README.hi.md)

Claude Code와 Codex CLI 계정을 버튼 하나로 갈아타는 데스크톱 위젯 (Windows·macOS).

<p align="center"><img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/screenshot.png" alt="switcher — Type 1 / 2 / 3" /></p>
<p align="center"><sub>세 가지 보기 모드 — Type 1 (전체) · Type 2 (위젯) · Type 3 (컴팩트)</sub></p>

## Windows

### 설치

**npm으로 설치 (권장 — 보안 경고 없음)** — Node.js 18 이상

```sh
npm install -g switcher-widget
switcher
```

`switcher` 명령이 처음 실행될 때 최신 릴리스 빌드를 자동으로 받아온다(이후에는 바로 뜬다). 브라우저 다운로드가 아니라서 SmartScreen 경고가 뜨지 않는다. 업데이트는 자동이다 — 실행할 때마다 새 릴리스를 확인해 다음 실행부터 반영된다.

**직접 다운로드** — [릴리스](https://github.com/Youkamii/switcher/releases/latest)에서 `switcher-win-x64.zip`을 받아 압축을 풀고 `switcher.exe`를 실행. (Windows 10/11 64비트)

- 코드 서명이 없어서 처음 실행할 때 Windows SmartScreen이 "알 수 없는 게시자" 경고를 띄울 수 있다. `추가 정보` → `실행`.
- 웹뷰는 Windows에 기본 포함된 WebView2를 사용한다.

### 실행

- 켜져 있는 동안은 트레이(작업표시줄 오른쪽)에 W 아이콘으로 상주한다. 창을 닫아도(Alt+F4) 꺼지지 않음.
- 창을 다시 활성화하려면 트레이의 W 아이콘을 좌클릭. 완전히 종료하려면 트레이 아이콘 우클릭 → 종료.
- UI 언어는 트레이 아이콘 우클릭 → 설정 → 언어에서 바꾼다 (한국어·English·日本語·简体中文·繁體中文·हिन्दी).
- 첫 실행 때 바탕화면에 `switcher` 바로가기가 자동으로 생긴다 (지우면 다시 만들지 않음).
- 부팅 시 자동 실행은 기본으로 켜져 있다 — 트레이 설정 → 부팅 시 자동 실행에서 끌 수 있다.
- 실행할 때마다 새 릴리스를 확인해 자동 업데이트한다 (다음 실행부터 반영) — 트레이 설정 → 자동 업데이트에서 끌 수 있다.

## macOS

### 설치

**npm으로 설치 (권장 — 보안 경고 없음)** — Node.js 18 이상

```sh
npm install -g switcher-widget
switcher
```

`switcher` 명령이 처음 실행될 때 최신 릴리스 빌드를 자동으로 받아온다(이후에는 바로 뜬다). 브라우저 다운로드가 아니라서 "확인되지 않은 개발자" 경고가 뜨지 않는다. 업데이트는 자동이다 — 실행할 때마다 새 릴리스를 확인해 다음 실행부터 반영된다.

**직접 다운로드** — [릴리스](https://github.com/Youkamii/switcher/releases/latest)에서 `switcher-mac-arm64.zip`을 받아 압축을 풀고 `switcher.app`을 실행. Apple Silicon 전용 — 인텔 맥은 아래 [직접 빌드](#직접-빌드)로 설치.

- 코드 서명이 없어서 처음 열 때 "확인되지 않은 개발자"라며 막힐 수 있다. 시스템 설정 → 개인정보 보호 및 보안 맨 아래에 나타나는 **그래도 열기**로 실행.

### 실행

- `switcher.app` 실행. Dock과 Cmd+Tab에는 나타나지 않고 메뉴바 오른쪽에 W 아이콘으로 상주.
- 위젯은 모든 데스크탑(Space)과 전체화면 앱 위에서 오버뷰로 표시된다.
- 창을 열고 숨기는 건 메뉴바 W 아이콘 좌클릭 토글, 완전히 종료하려면 우클릭 → 종료.
- UI 언어는 메뉴바 W 아이콘 우클릭 → 설정 → 언어에서 바꾼다 (한국어·English·日本語·简体中文·繁體中文·हिन्दी).
- 부팅 시 자동 실행은 기본으로 켜져 있다 — 트레이 설정 → 부팅 시 자동 실행에서 끌 수 있다 (시스템 설정 → 로그인 항목에도 표시된다).
- 실행할 때마다 새 릴리스를 확인해 자동 업데이트한다 (다음 실행부터 반영) — 트레이 설정 → 자동 업데이트에서 끌 수 있다.

## 위젯 사용법 (Windows·macOS 공통)

<table align="center">
<tr>
<td align="center" width="450">
<img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/demo.gif" width="420" alt="위젯 모드 데모 — 계정 카드 더블클릭 전환, 빈 영역은 뒤 창으로 클릭 통과" />
</td>
<td width="430">

**위젯 모드 동작**

- 계정 카드를 **더블클릭** → 해당 계정으로 인증 전환
- 카드 밖 클릭·드래그는 **뒤 창으로 그대로 통과**
- 활성화된 계정은 높은 채도로 표시 됨
- 창 이동은 ☰ 핸들, 모드 순환은 오른쪽 위 Type 버튼
- 맥에서는 모든 데스크탑(Space)과 전체화면 앱 위에서도 오버뷰

</td>
</tr>
</table>

## 개요

Claude Code든 Codex든 한 터미널에서 사용할 때, 한 계정만 로그인된다. 다계정 유저는 한도가 찰 때마다 `/login`을 다시 하고, 브라우저 인증을 다시 거치고, 지금 어느 계정을 쓰고 있는지도 헷갈린다.

switcher는 이 과정을 없앤다. 계정마다 처음 한 번만 로그인해 두면, 그다음부터는 위젯에서 버튼 한 번으로 전환된다. 각 계정의 사용량(5시간·주간 한도)이 막대로 보이니, 어느 계정에 여유가 있는지 보고 갈아타면 된다.

## 기능

- 계정 전환: 재로그인 없이 버튼 한 번. 새로 여는 터미널부터 적용된다.
- 사용량 표시: 계정마다 5 Hours / Weekly / 모델별 한도와 리셋까지 남은 시간이 보인다.
- 계정 추가: 위젯에 표시되는 로그인 링크에서 코드 발급 후 입력한다.
- 구독 레벨: 계정 옆에 Max(5x는 노랑, 20x는 빨강) / Pro / Plus가 붙는다.
- 모드(Type1/2/3): 전체 → 위젯 → 컴팩트 순환. 위젯·컴팩트에서는 버튼이 숨고 클릭·드래그가 뒤 창으로 통과하며, 계정 카드를 더블클릭하면 전환된다. 창 이동은 ☰ 핸들.
- 창 높이는 내용에 맞춰 자동 조절된다. 투명도 슬라이더를 내리면 배경이 먼저, 골조가 나중에 옅어진다.
- UI 언어: 트레이 → 설정 → 언어에서 6개 언어(한국어·영어·일본어·간체중문·번체중문·힌디) 전환.
- 자동 업데이트·부팅 시 자동 실행: 트레이 설정에서 켜고 끈다. 바탕화면 바로가기는 Windows 전용.
- GitHub 계정 전환: gh CLI에 로그인된 계정들을 위젯에서 전환 — git push/pull(HTTPS)이 활성 계정을 따라간다. 사용량 표시는 없음.
- 블랙 모니터: 🌙 버튼 또는 트레이 메뉴로 모든 화면을 최상위 검은 막으로 덮는다. 마우스를 움직이면 주변만 연기 걷히듯 비치고, 마우스를 1~2초 세게 흔들거나 ESC로 해제 — 마지막 커서 자리에서 빛이 퍼지며 걷힌다. macOS는 전체화면 앱 위는 덮지 못한다.
- 계정 정보 가리기: 🙈 버튼으로 카드의 이메일·GitHub 계정명을 블러 처리 — 화면 공유·스크린샷 노출 방지. 다시 누르면 해제.
- 화면 밝기 조절: DISPLAY 섹션의 모니터별 슬라이더로 실제 백라이트를 조절한다 (Windows는 DDC/CI, macOS는 내장 디스플레이). DDC/CI가 꺼진 모니터와 맥의 외장 모니터는 미지원 안내가 뜬다.
- TFSD (Token Full Self-Driving): 활성 계정의 사용량 창(5 Hours·Weekly·Fable 등) 중 하나라도 90%에 닿으면, 모든 창에 여유가 있는 계정 중 병목이 가장 낮은 곳으로 자동 전환. 꽉 찬 창이 30분 안에 리셋되면 전환 대신 대기. 트레이 설정에서 켠다 (기본 꺼짐).

## 동작

두 CLI 모두 로그인 토큰을 로컬에 저장한다.

- Claude Code: `~/.claude/.credentials.json` (Windows) / macOS는 **키체인**의 "Claude Code-credentials" 항목
- Codex CLI: `~/.codex/auth.json` (두 OS 동일)

맥에서 switcher는 클로드 CLI와 같은 방식(macOS 내장 `security` 도구)으로 키체인을 읽고 쓴다 — 별도 권한 팝업 없이 동작한다.

switcher는 계정별 토큰을 `~/.switcher/` 아래 프로필로 보관하고 전환할 때 두 단계로 파일을 교체한다.

1. 지금 활성 파일을 현재 계정 프로필에 백업한다. 토큰이 수시로 자동 갱신되므로 이 순서가 먼저여야 한다.
2. 대상 계정 프로필을 활성 위치로 복사한다.

주의: 터미널에서 CLI 세션이 돌아가는 중이라면 끝내고 전환하는 게 안전하다. 켜둔 세션이 토큰을 자동 갱신하면서 활성 파일을 다시 쓰면, 방금 전환한 계정이 이전 계정 토큰으로 덮일 수 있다.

대화 기록·메모리·설정은 계정과 무관한 로컬 폴더에 있어서 계정을 바꿔도 작업 환경은 그대로다.

사용량은 각 계정의 토큰으로 CLI가 쓰는 사용량 API를 직접 조회한다. 요청 제한을 피하려고 60초 캐시를 둔다. 조회가 막히면 직전 값을 보여준다.

클로드 액세스 토큰은 수명이 몇 시간뿐이라, 보관함 프로필의 토큰이 만료되면 위젯이 CLI와 같은 방식으로 재발급해 프로필에 되쓴다 — 앱을 켤 때 한 번 전체를, 이후엔 조회할 때 필요한 것만. 그래서 안 쓰는 계정의 사용량도 계속 실시간이다. 지금 쓰는 계정의 토큰은 CLI가 스스로 갱신하므로 위젯이 건드리지 않는다.

계정 추가는 격리 로그인으로 처리한다.

## 계정 추가

위젯의 "＋ 계정 추가"를 누르면 로그인 주소가 나온다. 그 주소를 원하는 브라우저에 붙여넣는다.

- **Claude**: 브라우저에서 로그인하면 화면에 코드가 나온다. 그 코드를 위젯 입력칸에 붙여넣으면 끝.
- **Codex**: 위젯에 주소와 함께 일회용 코드(15분 유효)가 뜬다. 브라우저에서 그 코드를 입력하면 나머지는 자동이다.

**Codex를 처음 추가하기 전에**: 장치 코드 인증이 OpenAI 계정에서 기본으로 꺼져 있다. 켜지 않으면 코드를 입력해도 "장치 코드 인증을 활성화한 뒤 다시 실행하세요"라며 거부된다.

- 개인 계정: chatgpt.com → 프로필 → 설정 → 보안(또는 데이터 제어) → **Codex 장치 코드 인증** 켜기
- 팀·비즈니스 계정: 관리자가 워크스페이스 설정 → 권한 및 역할에서 활성화

참고: Claude CLI는 로그인을 시작할 때 기본 브라우저를 한 번 열려고 한다. 그 창은 닫아도 되고, 위젯의 주소를 붙여넣은 브라우저에서 진행하면 된다.

## GitHub 계정 전환

[GitHub CLI(gh)](https://cli.github.com)가 설치되어 있으면 위젯에 GITHUB 섹션이 나타난다. 계정 추가는 위젯의 "＋ 계정 추가" 버튼으로 한다 — 주소와 일회용 코드가 뜨고, 브라우저에서 코드를 입력하면 끝 (터미널 `gh auth login`도 그대로 동작). 추가 후에는 위젯에서 전환된다 — 내부적으로 `gh auth switch`와 같은 통로를 쓰고, 전환할 때마다 `gh auth setup-git`을 실행해 git push/pull(HTTPS)이 활성 계정을 따라가게 한다. 토큰은 gh가 keyring에 관리하며 위젯은 만지지 않는다.

알아둘 한계:

- SSH 리모트(`git@github.com:...`)는 SSH 키가 신원을 정하므로 이 전환의 영향을 받지 않는다. HTTPS 리모트만 해당.
- 커밋 작성자(`git config user.name/email`)는 바뀌지 않는다 — 전환해도 커밋에는 기존 이름이 남는다.
- VS Code·Copilot 등 다른 앱의 GitHub 세션은 자체 토큰이라 따라오지 않는다.
- SAML SSO를 쓰는 조직 저장소는 계정별로 SSO 승인이 있어야 접근된다.
- 계정 추가·전환 시 실행되는 `gh auth setup-git`은 전역 git 설정에 github.com용 credential helper(gh)를 영구 등록한다 — 기존 GCM 설정을 대체하며, 되돌리려면 `git config --global --unset-all credential.https://github.com.helper`.

## 기술

Tauri 2 + Rust, 프론트는 바닐라 TypeScript. 계정 전환·사용량 조회·격리 로그인은 전부 Rust에서 처리한다.
웹뷰에는 토큰이 올라가지 않는다.
CLI 로그인 화면은 가상 콘솔(PTY)로 읽는다.

## 직접 빌드

받아서 쓰는 대신 소스에서 빌드하려면 [Node.js](https://nodejs.org)와 [Rust](https://rustup.rs) 툴체인이 필요하다.

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm run setup
```

`npm run setup`이 의존성 설치와 앱 빌드를 한 번에 처리한다. 장황한 로그를 쏟아내는 대신 로딩 표시와 경과 시간만 보여준다.

처음에는 Rust를 통째로 컴파일하기 때문에 **5~10분 걸릴 수 있다.** 로딩이 멈춘 게 아니니 기다리면 된다. 결과물은 Windows `src-tauri\target\release\switcher.exe`, macOS `src-tauri/target/release/bundle/macos/switcher.app` — 앱을 응용 프로그램 폴더로 옮겨도 된다.

개발 실행은 `npm run tauri dev`.

---

<div align="center">
<sub>Licensed under the <a href="LICENSE">MIT License</a> — free for any use, including commercial. Keep the copyright and license notice.</sub>
</div>
