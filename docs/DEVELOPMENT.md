# 개발·배포 안내

[README로 돌아가기](../README.md)

아래 명령은 저장소 루트에서 실행합니다. 사용자용 설치 안내와 화면은 [README](../README.md)에 있습니다.

## 개발 환경에서 실행

Windows에서 다음 도구가 필요합니다.

- Rust **1.99.1**, Visual Studio C++ Build Tools의 MSVC 도구 모음
- [MiKTeX](https://miktex.org/download) 또는 [TeX Live](https://www.tug.org/texlive/)
- PDF 미리보기용 [Poppler](https://poppler.freedesktop.org/)의 `pdftoppm`

Rust 버전은 `rust-toolchain.toml`에, Rune은 `Cargo.toml`의 커밋 `a05683be96cbd725ca1fbb34810368309ac534a5`에 고정되어 있습니다. 처음 빌드할 때 의존성을 내려받으므로 인터넷 연결이 필요합니다.

```powershell
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
.\scripts\Run-RuneTeX.ps1
```

앱을 직접 실행하면 Windows의 사용자 문서 폴더 아래 `RuneTeX/research-paper`에 기본 프로젝트를 만듭니다. OneDrive 등으로 이동된 문서 폴더도 따릅니다. 개발용 `Run-RuneTeX.ps1`은 기존 작업을 유지하도록 저장소의 `projects/research-paper`를 명시적으로 엽니다. 기존 프로젝트 폴더 또는 `.tex` 파일을 지정할 수도 있습니다.

기본 예제는 본문을 `main.tex` 하나에 모으고 `references.bib`만 별도로 둡니다. 문서 개요는 제목 계층에 따라 `1`, `2`, `2.1`, `3`으로 표시합니다. 긴 문서는 필요할 때 파일을 나누고 `\input`이나 `\include`로 연결할 수 있습니다.

```powershell
.\scripts\Run-RuneTeX.ps1 -Project "C:\Documents\paper\main.tex"

# Cargo로 직접 실행
cargo run --locked -- --project "C:\Documents\paper\main.tex"
```

실행 스크립트는 변경 사항을 증분 빌드하고 앱을 엽니다. `-Release`를 붙이면 최적화 빌드를 사용합니다. 기본 엔진은 **XeLaTeX**입니다. 문서는 **UTF-8**로 저장하며 기본 템플릿은 `kotex`를 사용합니다. Windows에서는 소스 편집기의 한글을 맑은 고딕으로 표시합니다. 기존 한글 문서에도 `\usepackage{kotex}`를 추가하세요. 엔진은 설정에서 변경할 수 있습니다.

LaTeX 언어 서버는 [texlab 5.26.0](https://github.com/latex-lsp/texlab/releases/tag/v5.26.0)을 사용합니다. 개발용 실행 스크립트는 서버가 없으면 공식 바이너리를 SHA-256 검증 후 `.deps/texlab`에 설치하고 해당 경로를 앱에 전달합니다. 직접 설치하려면 `scripts/Install-Texlab.ps1`을 실행하세요. `RUNETEX_TEXLAB`으로 별도 실행 파일을 지정할 수도 있습니다. 설정에서 연결 상태를 확인하거나 재연결할 수 있습니다.

입력은 LSP에 동기화되며 자동완성, 진단 밑줄과 문제 패널, 명령 설명 및 프로젝트 내 정의 이동을 지원합니다. `\input`·`\include`로 연결된 파일과 BibTeX 파일도 서버에 전달합니다. texlab이 별도의 컴파일을 실행하지 않도록 설정하고 PDF 컴파일은 RuneTeX가 관리합니다. 서버 로그는 프로젝트의 `.runetex/texlab.log`에 저장합니다.

PDF 미리보기에는 Poppler의 `pdftoppm`과 `pdfinfo`가 필요합니다. `pdftoppm`은 PATH 또는 환경 변수로 지정하며, `pdfinfo`는 같은 폴더에서 찾습니다. 별도 위치라면 `RUNETEX_PDFINFO`로 지정할 수 있습니다. 실행 스크립트는 설치된 Codex 런타임에 Poppler가 있으면 자동으로 찾습니다.

```powershell
$env:RUNETEX_PDFTOPPM = "C:\Tools\poppler\Library\bin\pdftoppm.exe"
.\scripts\Run-RuneTeX.ps1
```

앱은 명시한 환경 변수, 실행 파일 옆에 동봉된 도구, 시스템 PATH 순서로 texlab·Poppler를 찾습니다. 동봉 경로는 `tools/texlab/texlab.exe`와 `tools/poppler/Library/bin/pdftoppm.exe`입니다(Poppler는 `tools/poppler/bin`도 지원). 이 경로는 작업 폴더가 달라도 실행 파일 위치를 기준으로 처리하며, 개발 PC의 저장소 위치를 사용하지 않습니다. TeX 엔진은 PATH와 표준 MiKTeX 설치 경로에서 찾습니다.

편집기 릴리스 빌드는 `cargo build --release --locked --bin runetex`로 생성합니다. Windows 릴리스 실행 파일은 일반 실행 시 콘솔 창을 열지 않으며, 파일 속성의 제품·버전 정보와 `--version` 출력은 Cargo의 패키지 버전을 따릅니다. 기본 문서와 사용자 설정은 설치 폴더 밖에 저장됩니다.

## 설치 파일 만들기

Windows x64 개발 환경에서 texlab과 완전한 Poppler Windows 배포본을 먼저 준비합니다. texlab은 `scripts/Install-Texlab.ps1`로 `.deps/texlab`에 준비할 수 있습니다. Poppler는 `Library/bin/pdftoppm.exe`, 같은 폴더의 DLL, 데이터와 라이선스를 포함한 배포 폴더를 지정해야 합니다. 현재 준비된 배포본은 [Poppler Windows 26.09.0-0](https://github.com/oschwartz10612/poppler-windows/releases/tag/v26.09.0-0)입니다.

```powershell
.\scripts\Build-Installer.ps1 -PopplerDirectory .deps\poppler\poppler-26.09.0
```

이 명령은 기본으로 `runetex`와 `runetex-setup`의 Release 바이너리를 빌드하고 다음 배포 파일을 생성합니다. 버전은 `Cargo.toml`을 따릅니다. PowerShell 5.1 이상을 지원하며 필요한 입력이 없으면 오류를 내고 중단합니다. 패키징 스크립트는 의존성을 내려받거나 설치하지 않습니다.

- `dist/RuneTeX-Setup-0.1.0-windows-x64.exe`
- `dist/RuneTeX-0.1.0-SHA256SUMS.txt`: 설치 EXE의 SHA-256

`-Configuration Debug`는 로컬 확인용 빌드를 사용합니다. 이미 빌드한 실행 파일은 `-EditorExecutable`과 `-SetupExecutable`로 지정할 수 있으며, 스크립트가 x64 형식과 제품 버전을 확인합니다. `-TexlabDirectory`로 별도 texlab 배포 폴더를 지정할 수 있습니다. 설치 stub은 서명되지 않은 실행 파일이어야 하며 최종 설치 파일의 서명·게시 작업은 이 스크립트가 수행하지 않습니다.

패키지는 앱 실행 파일, `tools/texlab`, `tools/poppler`, 루트 문서와 배포 고지를 담습니다. 저장소의 `licenses/`에는 원문과 빌드 레시피를 담은 `notices.zip`, 그 체크섬 `notices.sha256`, 읽을 수 있는 `SOURCES.md` 세 파일을 보관합니다. 빌드할 때 검증된 원문을 `.cache/notices/`에 자동으로 풀고 설치 파일에는 기존 `licenses/` 경로로 포함합니다. 압축을 푼 캐시는 Git에서 제외됩니다. 소스 링크는 [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md)와 [소스 목록](../licenses/SOURCES.md)에도 정리되어 있습니다. 빌드에 성공하면 같은 버전의 이전 `SOURCES.md`·`LICENSES.zip`·중복 `.exe.sha256` 산출물을 정리합니다.

GitHub Release에는 위 두 파일을 첨부합니다. 변경 사항은 [CHANGELOG.md](../CHANGELOG.md)의 해당 버전에 작성하고, 다음 명령으로 변경 사항과 동봉 의존성의 원본 소스·빌드 자료 링크를 포함한 본문을 클립보드에 복사합니다.

```powershell
.\scripts\Write-ReleaseNotes.ps1 | Set-Clipboard
```

복사한 Markdown을 GitHub Release의 설명란에 붙여 넣습니다. 버전은 `Cargo.toml`을 따르며 해당 버전의 변경 사항이 없으면 본문 생성을 중단합니다. 본문에는 저장소 주소에 의존하지 않는 원본 다운로드 링크를 넣어, 배포 파일과 함께 소스 위치를 확인할 수 있게 합니다.

패키징 전에 압축 묶음의 SHA-256과 원문을 확인하고, 현재 `Cargo.lock`·`Cargo.toml`·Rust 도구 체인 및 texlab·Poppler 파일과 대조합니다. 의존성이 달라지거나 원문이 빠지면 빌드를 중단합니다. Rust 의존성이 바뀌면 `scripts/Generate-RustNotices.py`로 캐시의 Windows 고지를 다시 만들고, texlab·Poppler가 바뀌면 해당 버전의 원본 패키지·소스·라이선스 목록도 갱신합니다. `scripts/Collect-PopplerNotices.py --check`는 현재 Poppler 고지를 검사합니다. 마지막 명령은 변경한 캐시와 저장소의 소스 안내를 다시 묶어 `notices.zip`·체크섬을 갱신합니다. 고지 생성에는 Python 3.11 이상이 필요합니다.

```powershell
python .\scripts\Generate-RustNotices.py
.\scripts\Update-NoticeManifest.ps1 -PopplerDirectory .deps\poppler\poppler-26.09.0
```

동봉한 texlab은 공식 5.26.0 실행 파일이며, [원본 소스](https://codeload.github.com/latex-lsp/texlab/zip/45c3ced6790b508a12a2aab1938e58023173017e)는 릴리스 커밋 `45c3ced6790b508a12a2aab1938e58023173017e`로 고정했습니다. 배포 페이지에서도 버전에 맞는 소스를 계속 받을 수 있도록 링크를 유지합니다.

설치 화면의 디자인만 확인하려면 명시적으로 미리보기 모드를 사용합니다.

```powershell
cargo run --locked --bin runetex-setup -- --preview
cargo run --locked --bin runetex-setup -- --screen progress
cargo run --locked --bin runetex-setup -- --screen complete
```

미리보기에서는 실제 설치나 제거를 실행하지 않습니다. 일반 실행의 단계 표시는 실제 작업 상태를 따릅니다. 설치 화면 디자인 검토에는 Damir Sokolovsky의 [unslop](https://github.com/heydamir/unslop) 지침(CC BY 4.0)을 사용했습니다.

## 설치·제거와 명령행 사용

`RuneTeX-Setup-0.1.0-windows-x64.exe`는 Rune으로 만든 네이티브 설치 프로그램입니다. 설치 경로와 바로가기를 선택하고 **설치**를 누르면 파일 검증·복사, 바로가기 생성, Windows 앱 등록이 실제 작업에 따라 진행됩니다. 현재 사용자에게 설치하므로 관리자 권한을 요구하지 않습니다. 설치 중 취소하면 이번 설치의 변경 사항을 되돌립니다.

설치 파일에는 RuneTeX, texlab, DLL과 인코딩 데이터를 포함한 Poppler 배포본이 들어 있습니다. 설치 과정에서 추가 구성 요소를 다운로드하지 않습니다. **MiKTeX 또는 TeX Live는 별도로 설치해야 합니다.** 편집기의 기본 엔진은 XeLaTeX입니다.

설치 후 Windows **설정 → 앱 → 설치된 앱**에서 RuneTeX를 제거할 수 있습니다. 제거 시 설치한 앱과 도구, 바로가기와 앱 등록을 정리하며 사용자 문서와 `%APPDATA%\RuneTeX`의 설정은 보존합니다.

설치 파일의 명령행 사용 예입니다. `--package-info`는 내용의 해시를 검증하고 버전·파일 수·크기를 출력하며, `--silent`는 화면 없이 실제 설치 또는 제거를 수행합니다.

```powershell
.\dist\RuneTeX-Setup-0.1.0-windows-x64.exe --package-info
.\dist\RuneTeX-Setup-0.1.0-windows-x64.exe --silent --destination "$env:LOCALAPPDATA\Programs\RuneTeX" --no-shortcuts
& "$env:LOCALAPPDATA\Programs\RuneTeX\uninstall.exe" --uninstall --silent
```

`--uninstall <설치 경로> --silent`로 제거할 설치 위치를 명시할 수도 있습니다.

## 편집 설정과 단축키

설정의 **UI 글꼴**과 **에디터 글꼴**은 각각 **검색 가능한 드롭다운 → 초기화 → 적용** 순서입니다. 목록의 각 이름은 해당 글꼴로 미리 표시됩니다. 선택한 뒤 적용하면 반영되며, 초기화는 선택값을 기본 글꼴로 돌립니다(UI: Segoe UI, 에디터: Cascadia Code). 두 글꼴은 독립적으로 변경되고 한글 대체 글꼴은 유지됩니다. Windows에서는 `%APPDATA%\RuneTeX\ui-font.json`과 `editor-font.json`에 앱 전체 설정으로 저장됩니다.

에디터 글자 크기는 설정의 ± 버튼 또는 `Ctrl++` / `Ctrl+-`로 1px씩 조절합니다(`Ctrl+=`도 확대). 범위는 8–48px이며, 글꼴과 크기는 재실행 후에도 유지됩니다.

설정은 **글꼴과 편집 → 컴파일 → 언어 서버 → 단축키**로 나뉩니다. 자동 컴파일과 자동 패키지 설치는 스위치로 상태를 표시하며, 완료 버튼은 스크롤 위치와 관계없이 하단에 표시됩니다. 작은 창에서는 에디터 글꼴 목록이 위로 열립니다.

소스와 PDF 사이의 분할선을 드래그하면 두 영역의 너비를 조절할 수 있습니다. 분할선에 키보드 초점이 있을 때 좌우 방향키로 조절하고 `Home`으로 기본 비율을 복원합니다.

- 탐색기와 탭으로 여러 파일을 편집하고, 새 `.tex`·`.bib`·`.sty` 파일을 만듭니다.
- 프로젝트 검색에서 파일과 내용을 찾고, 문서 개요에서 해당 소스로 이동합니다.
- 컴파일 버튼 또는 `Ctrl+Enter`로 저장 후 PDF를 생성합니다. 설정에서 자동 컴파일을 켤 수 있습니다.
- PDF 미리보기에서 페이지를 바꾸거나 배율을 조절하고, 생성한 PDF를 내보냅니다.
- 문제 패널에서 오류의 파일·줄 위치를 확인하고, 출력 패널에서 컴파일 로그를 봅니다.

| 단축키              | 동작                    |
| ------------------- | ----------------------- |
| `Ctrl+S`            | 저장                    |
| `Ctrl++` / `Ctrl+=` | 에디터 글자 크기 늘리기 |
| `Ctrl+-`            | 에디터 글자 크기 줄이기 |
| `Ctrl+Enter`        | 컴파일                  |
| `Ctrl+K`            | 프로젝트 검색           |
| `Ctrl+Space`        | texlab 자동완성         |
| `Tab` / `Enter`     | 선택한 자동완성 적용    |
| `F12`               | 정의로 이동             |
| `F1`                | 명령 설명               |
| `Ctrl+B`            | 탐색기 열기·닫기        |
| `Esc`               | 열린 대화상자 닫기      |

## 컴파일 범위

TeX 엔진을 두 번 실행해 교차 참조를 갱신합니다. BibTeX·Biber·외부 빌드 도구의 자동 실행은 지원하지 않습니다. 설정의 **자동 패키지 설치**는 기본으로 켜져 있으며 MiKTeX에서 누락된 패키지를 내려받습니다. 설정은 프로젝트의 `.runetex/settings.conf`에 저장되어 재실행 후에도 유지됩니다. TeX Live의 패키지는 별도로 설치해야 하며, `PRIMEarxiv.sty` 같은 사용자 템플릿 파일은 프로젝트에 직접 넣어야 합니다. TeX의 shell escape는 끕니다.

각 빌드 결과는 프로젝트의 `.runetex/build` 아래 별도 폴더에 저장됩니다. TeX 실행은 회당 90초이며 MiKTeX 자동 설치를 켰을 때에는 다운로드를 포함해 최대 10분을 허용합니다. PDF 미리보기에는 페이지 수 제한이 없습니다. 먼저 전체 페이지 수와 크기만 읽고, 별도 작업 스레드에서 화면에 보이는 페이지를 우선 변환한 뒤 앞뒤 두 페이지를 미리 불러옵니다. 변환은 페이지별 최대 60초를 허용합니다. 이미지 파일은 해당 빌드 폴더에 캐시하며, 메모리에는 최근 페이지를 64MiB 기준으로 보관합니다. 현재 보이는 페이지는 메모리 예산을 넘어도 유지합니다. Poppler가 없어도 PDF 컴파일과 내보내기는 가능하며, 메타데이터 문제는 출력 패널에, 개별 페이지의 변환 실패는 해당 페이지에 표시합니다.

참고: [Rune 공식 저장소](https://github.com/anomalyco/rune).
