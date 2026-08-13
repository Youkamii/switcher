<h1 align="center">
  <a href="https://github.com/Youkamii/switcher/releases/latest">
    <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/readme-hero.svg" width="100%" alt="switcher — Claude Code, Codex CLI, GitHub CLI 계정 전환 위젯" />
  </a>
</h1>

<p align="center">
  <a href="https://github.com/Youkamii/switcher/releases/latest"><img src="https://img.shields.io/github/v/release/Youkamii/switcher?style=flat-square&label=release&color=8B5CF6" alt="latest release" /></a>
  <a href="https://www.npmjs.com/package/switcher-widget"><img src="https://img.shields.io/npm/v/switcher-widget?style=flat-square&color=CB3837" alt="npm version" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/Youkamii/switcher?style=flat-square&color=22C55E" alt="MIT License" /></a>
  <a href="https://github.com/Youkamii/switcher/stargazers"><img src="https://img.shields.io/github/stars/Youkamii/switcher?style=flat-square&color=F59E0B" alt="GitHub stars" /></a>
</p>

<p align="center">
  <strong>Claude Code · Codex CLI · GitHub CLI 계정을 한 위젯에서 전환하세요.</strong><br />
  다시 로그인하지 않고 사용량과 리셋 시간을 확인하고, TFSD를 켜면 한도에 닿기 전에 다음 계정으로 넘어갑니다.
</p>

<p align="center">
  <a href="#30초-설치"><strong>30초 설치</strong></a> ·
  <a href="#주요-기능">주요 기능</a> ·
  <a href="#지원-범위와-배포">지원 범위</a> ·
  <a href="#데이터와-보안">데이터와 보안</a> ·
  <a href="#개발과-기여">기여하기</a>
</p>

<p align="center">
  <strong>한국어</strong> ·
  <a href="docs/README.en.md">English</a> ·
  <a href="docs/README.ja.md">日本語</a> ·
  <a href="docs/README.zh-CN.md">简体中文</a> ·
  <a href="docs/README.zh-TW.md">繁體中文</a> ·
  <a href="docs/README.hi.md">हिन्दी</a>
</p>

## 30초 설치

Node.js 18 이상에서 아래 두 줄이면 끝입니다.

```sh
npm install -g switcher-widget
switcher
```

첫 실행 때 설치한 npm 패키지와 **같은 버전**의 공식 릴리스 파일을 내려받습니다. 사용하려는 [Claude Code](https://docs.anthropic.com/en/docs/claude-code) 또는 [Codex CLI](https://github.com/openai/codex)는 별도로 설치되어 있어야 합니다.

<p align="center">
  <a href="https://github.com/Youkamii/switcher/releases/latest/download/switcher-win-x64.zip"><strong>Windows x64 다운로드</strong></a>
  &nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="https://github.com/Youkamii/switcher/releases/latest/download/switcher-mac-arm64.zip"><strong>macOS Apple Silicon 다운로드</strong></a>
  &nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="https://github.com/Youkamii/switcher/releases/latest">릴리스 노트</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/screenshot.png" width="920" alt="switcher의 Type 1, Type 2, Type 3 화면" />
  <br />
  <sub>전체 제어 화면부터 폭 120px 미니 위젯까지, 필요한 만큼만 남깁니다.</sub>
</p>

## 한 번 로그인하고, 계속 바꾸세요

Claude Code와 Codex CLI는 한 번에 한 계정만 활성화합니다. 계정이 여러 개면 한도가 찰 때마다 로그아웃하고 브라우저 인증을 다시 거쳐야 하고, 어느 계정에 여유가 남았는지도 따로 확인해야 합니다.

switcher는 CLI가 쓰는 로컬 인증 저장소를 계정별로 관리합니다. 로그인은 계정마다 한 번만 하고, 그다음부터는 위젯에서 전환합니다.

<table>
  <tr>
    <td width="33%" valign="top">
      <strong>⚡ 한 번에 전환</strong><br /><br />
      현재 인증을 먼저 백업한 뒤 선택한 프로필을 활성화합니다. 새 터미널부터 바로 적용됩니다.
    </td>
    <td width="33%" valign="top">
      <strong>◉ 한도를 한눈에</strong><br /><br />
      활성·비활성 계정의 사용량, 구독 등급, 실제 리셋까지 남은 시간을 함께 보여 줍니다.
    </td>
    <td width="33%" valign="top">
      <strong>↗ 화면 위에 상주</strong><br /><br />
      전체·컴팩트·미니멀 모드와 클릭 통과를 조합해 작업 화면을 가리지 않습니다.
    </td>
  </tr>
</table>

## 주요 기능

### 계정 전환과 자동 사용량 갱신

- **Claude Code · Codex CLI** 프로필을 같은 화면에서 추가·삭제·전환
- 공급자가 제공하는 **5시간·주간·모델별 사용량 창**과 리셋 시간 표시
- 비활성 프로필도 토큰을 갱신해 사용량을 계속 업데이트
- Type 1은 버튼, Type 2·3은 카드 더블클릭으로 전환
- 이메일과 GitHub 계정명을 흐리는 화면 공유용 가리기

Type 2의 리셋 시간은 좁은 폭에 맞춰 압축됩니다. 24시간 미만은 `시:분`(예: `2:21`), 그 이상은 `일::시`(예: `5::17`)입니다.

### 세 가지 밀도, 하나의 위젯

오른쪽 위 Type 버튼으로 보기 모드를 순환합니다. Type 1에서는 CLAUDE·CODEX·GITHUB·DISPLAY·SYSTEM 섹션을 끌어 원하는 순서로 배치할 수 있습니다.

| 모드 | 폭과 역할 | 조작 |
| --- | --- | --- |
| **Type 1** | 계정 추가·삭제와 모든 도구가 보이는 전체 화면 | 버튼으로 전환 |
| **Type 2** | 이메일과 구독 정보를 남긴 컴팩트 위젯 | 카드 더블클릭 |
| **Type 3** | 라벨과 사용량 막대만 보이는 폭 120px 위젯 | 카드 더블클릭 |

<table>
  <tr>
    <td width="58%" align="center">
      <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/size-compare.png" width="610" alt="Codex 데스크톱 앱 펫과 switcher Type 3의 최소 크기 비교" />
    </td>
    <td width="42%" align="center">
      <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/transparency.gif" width="390" alt="코드 편집기 위에서 switcher 투명도를 조절하는 모습" />
    </td>
  </tr>
  <tr>
    <td align="center"><sub>Codex 데스크톱 앱 펫과 Type 3 최소 크기 비교</sub></td>
    <td align="center"><sub>배경부터 그래프까지 단계적으로 조절되는 투명도</sub></td>
  </tr>
</table>

Type 2·3의 빈 영역은 클릭과 드래그가 뒤 창으로 통과합니다. 최저 투명도에서는 사용량 그래프만 남겨 편집기 위에 겹칠 수 있습니다.

### TFSD 자율주행

TFSD(Token Full Self-Driving)는 활성 계정의 사용량 창 중 하나가 90%에 도달하면, **모든 사용량 창에 여유가 있는 프로필 중 가장 넉넉한 곳**으로 자동 전환합니다.

<table>
  <tr>
    <td width="42%" align="center">
      <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/tfsd.png" width="340" alt="TFSD가 켜져 활성 카드에 T 워터마크가 표시된 화면" />
    </td>
    <td width="58%" valign="middle">
      <strong>🚗 켜 두면 알아서 다음 계정으로</strong><br /><br />
      · 타이틀바 또는 트레이 설정에서 켜기<br />
      · 활성 카드의 T 워터마크로 상태 확인<br />
      · 90%를 넘긴 창들이 모두 30분 안에 리셋되면 전환하지 않고 대기<br />
      · 사용자가 직접 계정을 바꾸면 즉시 해제
    </td>
  </tr>
</table>

전환 기록은 `~/.switcher/tfsd-history.log`에 남습니다. 기록에는 프로필 이름이나 이메일이 평문으로 포함될 수 있습니다.

### 작업을 끊지 않는 화면 도구

**블랙 모니터**는 화면을 검은 오버레이로 덮습니다. 커서를 움직이면 주변만 연기처럼 걷히고, 마우스를 1~2초 세게 흔들거나 `Esc`를 누르면 해제됩니다. Windows는 DDC/CI 밝기 조절을 지원하는 모니터의 하드웨어 밝기도 함께 낮춥니다. macOS는 오버레이만 사용하며 전체 화면 앱이 열린 별도 Space는 덮지 못합니다.

<p align="center">
  <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/black.gif" width="720" alt="블랙 모니터에서 커서 주변이 드러났다가 해제되는 모습" />
</p>

<table>
  <tr>
    <td width="52%" align="center">
      <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/system.png" width="440" alt="CPU, 메모리, 디스크, 네트워크를 표시하는 SYSTEM 섹션" />
    </td>
    <td width="48%" align="center">
      <img src="https://raw.githubusercontent.com/Youkamii/switcher/main/docs/memo.png" width="300" alt="탭 5개와 투명도 조절이 있는 메모장" />
    </td>
  </tr>
  <tr>
    <td align="center"><strong>SYSTEM</strong><br /><sub>CPU · 메모리 · 디스크 · 네트워크와 60초 그래프</sub></td>
    <td align="center"><strong>MEMO</strong><br /><sub>자동 저장되는 5개 탭과 독립 투명도</sub></td>
  </tr>
</table>

### 기능 지도

| 기능 | 동작 | Windows | macOS |
| --- | --- | :---: | :---: |
| Claude · Codex 전환 | 로컬 프로필 백업 후 활성 인증 교체 | ✓ | ✓ |
| 사용량과 TFSD | 비활성 계정까지 갱신, 90%에서 자동 전환 | ✓ | ✓ |
| GitHub 전환 | `gh` 계정 전환과 HTTPS Git 인증 연결 | ✓ | ✓ |
| 클릭 통과·투명도 | 위젯 아래 앱을 그대로 조작 | ✓ | ✓ |
| DISPLAY | DDC/CI 외장 모니터 | ✓ | — |
| DISPLAY | 내장 디스플레이 밝기 | — | ✓ |
| 블랙 모니터 | 오버레이, 흔들기·`Esc` 해제 | 모든 모니터 | 전체 화면 Space 제외 |
| 클램셸 슬립 방지 | 덮개를 닫아도 터미널 작업 유지 | ✓ | ✓ |
| 자동 업데이트 | GitHub Releases에서 내려받아 적용 | ✓ | 구현·배포됨¹ |
| 자동 실행 | 트레이에서 설정 | ✓ | macOS 13+ |
| 다국어 UI | 한국어·영어·일본어·중국어 간체·번체·힌디 | ✓ | ✓ |

¹ macOS 자동 업데이트 코드는 릴리스에 포함되어 있지만, 최신 경로의 실제 Mac 업데이트 완료 검증은 아직 남아 있습니다.

#### Windows · macOS 클램셸 모드

☕를 한 번 누르면 다음 덮개 열림까지, 두 번 누르면 감시 프로세스가 살아 있는 동안 앱을 다시 실행해도 계속 유지됩니다. 정상 종료·재부팅 때 원래 설정을 복원하며, 감시 프로세스가 비정상 종료되면 다음 Switcher 실행 때 복구하고 기능을 끕니다.

- **Windows:** 현재 전원 구성표의 AC·배터리 덮개 동작을 각각 보관한 뒤 `아무 작업 안 함`으로 바꿉니다. 켜진 동안 구성표가 바뀌면 새 구성표도 별도로 보관하며, 해제할 때 사용자가 선택한 구성표는 유지하고 Switcher가 바꾼 값만 되돌립니다.
- **macOS:** `SleepDisabled`를 보관·복원하며, 켤 때 관리자 승인을 한 번 요청합니다.

#### GitHub 계정 전환

[GitHub CLI](https://cli.github.com)에 로그인된 `github.com` 계정을 전환합니다. HTTPS 리모트를 위해 `gh auth setup-git`을 사용하므로 GitHub CLI의 전역 HTTPS 자격증명 연결에도 반영됩니다. SSH 리모트, `git config user.name/email`, VS Code·Copilot 로그인은 바뀌지 않습니다. GitHub Enterprise 호스트는 현재 대상이 아닙니다.

## 처음 설정하기

### Claude · Codex 계정 추가

1. Type 1에서 Claude 또는 Codex의 **+ 계정 추가**를 누릅니다.
2. 위젯에 표시된 주소를 브라우저에서 엽니다.
3. Claude는 브라우저에 표시된 코드를 위젯 입력칸에 붙여넣습니다.
4. Codex는 브라우저에서 15분 유효 일회용 코드를 입력합니다.
5. 로그인이 끝나면 카드가 추가됩니다. 현재 활성 계정은 바뀌지 않습니다.

Codex 장치 코드 인증이 꺼져 있으면 로그인할 수 없습니다. 개인 계정은 ChatGPT **설정 → 보안 → Codex 장치 코드 인증**, 팀·비즈니스 계정은 관리자의 워크스페이스 권한 설정에서 켜세요.

### GitHub 계정 추가

[GitHub CLI](https://cli.github.com)가 설치되어 있으면 GITHUB 섹션이 나타납니다. **+ 계정 추가**를 누르고 브라우저에서 장치 코드를 승인하세요.

## 데이터와 보안

switcher 전용 계정 중계 서버는 없습니다. 앱은 로컬 CLI 인증 저장소를 읽고 쓰며, 사용량 조회와 토큰 갱신은 Anthropic 또는 OpenAI의 서비스에 직접 요청합니다. GitHub 인증은 `gh`가 관리하고, 업데이트는 GitHub Releases에서 받습니다.

> [!IMPORTANT]
> 계정 프로필에는 실제 인증 자격증명이 들어 있습니다. 프로필 복사본은 앱 전용 암호화 없이 로컬 파일로 저장되며, Unix 계열에서는 만들 때 `0600` 권한을 적용합니다. `~/.switcher`, `~/.claude`, `~/.codex` 안의 인증 파일을 Issue나 로그에 첨부하지 마세요.

| 대상 | 위치 |
| --- | --- |
| Claude 활성 인증 · Windows | `~/.claude/.credentials.json` |
| Claude 활성 인증 · macOS | 키체인의 `Claude Code-credentials`. CLI 호환을 위해 파일을 함께 쓸 수 있음 |
| Codex 활성 인증 · Windows/macOS | `~/.codex/auth.json` |
| Claude 프로필 복사본 | `~/.switcher/claude/profiles/<name>/` |
| Codex 프로필 복사본 | `~/.switcher/codex/profiles/<name>/` |
| TFSD 전환 기록 | `~/.switcher/tfsd-history.log` |

계정 전환 순서는 일부러 고정되어 있습니다.

1. 현재 활성 인증을 현재 프로필에 먼저 백업
2. 선택한 프로필을 활성 위치로 복사

이 순서를 지켜야 CLI가 자동 갱신한 최신 토큰을 잃지 않습니다. 토큰 값은 앱 로그와 오류 메시지에 출력하지 않습니다.

대화 기록, 메모리, 프로젝트 설정은 인증 파일과 별개라 계정을 바꿔도 그대로 유지됩니다. 이미 실행 중인 Claude Code·Codex 세션은 시작할 때 읽은 인증을 계속 쓸 수 있으므로, 전환 후 새 터미널 세션을 여는 것이 가장 확실합니다.

## 지원 범위와 배포

| 대상 | 공식 배포 파일 | 상태 |
| --- | --- | --- |
| Windows 10 1803+/11 x64 | `switcher-win-x64.zip` | 지원 |
| macOS Apple Silicon | `switcher-mac-arm64.zip` | 지원 |
| Windows ARM64 | x64 에뮬레이션 가능 환경 | 공식 실기기 검증 대상 아님 |
| macOS Intel | 없음 | 공식 지원 안 함 |
| Linux | 없음 | 공식 지원 안 함 |

### 배포 방식

- **npm**: `switcher-widget` 설치 후 첫 실행 때 패키지와 같은 버전의 위 파일을 받습니다.
- **직접 다운로드**: [최신 GitHub Release](https://github.com/Youkamii/switcher/releases/latest)에서 압축 파일을 받아 실행합니다.
- **소스 빌드**: 아래 개발 명령으로 현재 운영체제에서 직접 빌드할 수 있습니다.

> [!WARNING]
> 현재 배포 파일은 Windows Authenticode 또는 macOS Developer ID로 코드 서명·공증되지 않았습니다. npm 설치와 자동 업데이트도 같은 릴리스 파일을 사용하므로 운영체제 경고가 나타날 수 있습니다. 출처가 `github.com/Youkamii/switcher`인지 확인하세요. npm 실행기와 자동 업데이터는 현재 암호학적 서명이나 공개 체크섬을 대조하지 않습니다. 자동 업데이터는 GitHub 출처, 파일 크기, 내부 버전을 확인합니다.

### 실행과 업데이트

- Windows는 알림 영역, macOS는 메뉴 막대에 **W** 아이콘으로 상주합니다.
- 창을 닫아도 앱은 종료되지 않습니다. 완전히 끄려면 트레이 메뉴의 **종료**를 사용하세요.
- 자동 업데이트는 새 버전을 받아 다음 실행부터 적용합니다.
- 트레이의 **업데이트 확인**은 적용 뒤 앱을 자동으로 다시 시작합니다.
- 언어, 자동 업데이트, 자동 실행, TFSD, 표시할 섹션을 트레이 설정에서 바꿀 수 있습니다.

## 문제 해결

<details>
<summary><strong>Windows 또는 macOS에서 처음 실행이 차단됩니다</strong></summary>

공식 릴리스 파일은 아직 코드 서명되지 않았습니다. Windows는 **추가 정보 → 실행**, macOS는 앱을 우클릭해 **열기**를 선택하거나 **시스템 설정 → 개인정보 보호 및 보안 → 그래도 열기**를 사용하세요. 다운로드 주소가 이 저장소의 GitHub Releases인지 먼저 확인하세요.
</details>

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

삭제는 보관함 사본만 지우고 로그인 자체는 남깁니다. 계정을 전환할 때 현재 로그인 계정을 자동 백업하므로 활성 계정 프로필은 다음 전환 때 다시 생길 수 있습니다. 완전히 정리하려면 먼저 다른 계정으로 전환한 뒤 삭제하세요.
</details>

## 개발과 기여

이제 정식 오픈소스입니다. 작은 버그 수정, 문서 보완, 번역, 플랫폼 실기기 검증까지 모두 환영합니다.

```sh
git clone https://github.com/Youkamii/switcher.git
cd switcher
npm ci
npm run tauri dev
```

| 작업 | 명령 |
| --- | --- |
| 프론트 좌표 회귀 테스트 | `npm test` |
| 프론트 빌드·타입 검사 | `npm run build` |
| Rust 빠른 검사 | `cd src-tauri && cargo check` |
| Rust 테스트 | `cd src-tauri && cargo test` |
| Windows 포터블 빌드 | `npm run tauri build -- --no-bundle` |
| macOS 앱 빌드 | `npm run tauri build -- --bundles app` |

결과물은 Windows에서 `src-tauri/target/release/switcher.exe`, macOS에서 `src-tauri/target/release/bundle/macos/switcher.app`에 생성됩니다.

기여할 때는 다음 세 가지만 지켜 주세요.

1. 큰 변경은 먼저 [Issue](https://github.com/Youkamii/switcher/issues)에서 방향을 맞춥니다.
2. Pull Request에 확인한 운영체제와 실행한 검사 명령을 적습니다.
3. 실제 토큰, 계정 파일, `~/.switcher` 내용은 커밋·스크린샷·Issue에 절대 넣지 않습니다.

[Issue 열기](https://github.com/Youkamii/switcher/issues/new/choose) ·
[Pull Request 보기](https://github.com/Youkamii/switcher/pulls) ·
[macOS 실기기 검증 체크리스트](docs/MAC_VALIDATION_PROMPT.md)

## 기술 구성

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/Rust-native-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/TypeScript-vanilla-3178C6?style=for-the-badge&logo=typescript&logoColor=white" alt="Vanilla TypeScript" />
  <img src="https://img.shields.io/badge/Vite-frontend-646CFF?style=for-the-badge&logo=vite&logoColor=white" alt="Vite" />
</p>

- 계정 전환, 로그인, 사용량 조회, 시스템 연동은 Rust 커맨드에서 처리
- Windows WebView2 · macOS WKWebView
- Claude는 PTY 로그인, Codex는 장치 코드 로그인
- 릴리스 태그로 Windows·macOS 파일과 npm 패키지를 자동 배포

---

<p align="center">
  <strong><a href="LICENSE">MIT License</a></strong> · 개인·상업용 모두 사용 가능
  <br /><br />
  switcher는 Anthropic, OpenAI, GitHub와 제휴하거나 이들이 보증한 제품이 아닌 독립 오픈소스 프로젝트입니다.
  <br />
  <sub>Built for people who keep too many terminals open.</sub>
</p>
