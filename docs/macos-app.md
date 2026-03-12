# macOS App (turm-macos)

## Overview

Swift/AppKit app using [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) for terminal rendering and PTY management. SwiftTerm's `LocalProcessTerminalView` handles the PTY internally — the same design choice as VTE on Linux: no custom PTY layer needed.

**Tech stack:** Swift 6, AppKit, SwiftTerm 1.11+, macOS 14+

**Build system:** Swift Package Manager (standalone, not in Cargo workspace)

---

## Architecture

### Linux vs macOS 비교

| 항목 | Linux | macOS |
|---|---|---|
| UI framework | GTK4 | AppKit (NSWindow/NSViewController) |
| Terminal widget | VTE4 (`vte4::Terminal`) | SwiftTerm (`LocalProcessTerminalView`) |
| PTY 관리 | VTE 내장 (`spawn_async`) | SwiftTerm 내장 (`startProcess`) |
| IPC | D-Bus + Unix socket | Unix socket |
| 메인 스레드 전달 | `glib::timeout_add_local` 폴링 | `DispatchQueue.main.sync` (동기) |
| 설정 파싱 | `toml` crate | 직접 구현 (simple line parser) |
| 테마 | `turm-core/theme.rs` | `Theme.swift` (mirrors Rust struct) |
| 분할 창 | GTK Paned | `NSSplitView` + `EqualSplitView` |

### 디렉토리 구조

```
turm-macos/
├── Package.swift                      # SwiftTerm 의존성 선언
├── run.sh                             # .app 번들 생성 후 실행
└── Sources/Turm/
    ├── TurmApp.swift                  # @main 진입점
    ├── AppDelegate.swift              # NSWindow 생성, 메뉴바, 소켓 커맨드 라우팅
    ├── TabViewController.swift        # 탭 목록 관리, PaneManager 배열
    ├── TabBarView.swift               # 커스텀 탭바 UI (테마 색상)
    ├── PaneManager.swift              # 단일 탭의 split-pane 트리 관리
    ├── SplitNode.swift                # N-ary 분할 트리 데이터 구조
    ├── TerminalViewController.swift   # SwiftTerm 래퍼, 쉘 실행, delegate
    ├── SocketServer.swift             # POSIX Unix socket 서버
    ├── Config.swift                   # config.toml 파서
    └── Theme.swift                    # 10개 내장 테마 (RGBColor, TurmTheme)
```

---

## 빌드 및 실행

```bash
# 개발 중 빠른 테스트 (메뉴바 이름이 올바르지 않을 수 있음)
cd turm-macos
swift run

# 제대로 된 .app 번들로 실행 (권장)
./run.sh
# → .build/debug/Turm.app 생성 후 open으로 실행

# 빌드만
swift build
```

`run.sh`는 매번 `Turm.app/Contents/Info.plist`를 포함한 번들을 새로 만들어서 `open`으로 실행합니다. Info.plist가 있어야 Dock 아이콘, 메뉴바 앱 이름 등이 정상 표시됩니다.

---

## 파일별 구현 세부사항

### TurmApp.swift

`@main`으로 진입점 선언. `NSApplication.shared`를 직접 다루어 `AppDelegate`를 설정하고 `app.run()`으로 이벤트 루프 시작.

### AppDelegate.swift (`@MainActor`)

- `TurmConfig.load()` → `TurmTheme.byName()` 순서로 설정·테마 로드
- `NSWindow` 생성 (1200×800, titled/closable/resizable/miniaturizable)
- `TabViewController`를 `window.contentViewController`로 설정
- 메뉴바: App(종료), Shell(탭/분할/전환), Find(검색), View(줌) 구성
- 소켓 서버 시작 후 `handleCommand(method:params:)`로 모든 커맨드 라우팅

**Find 메뉴 (in-terminal search):**
`performFindPanelAction(_:)` 메서드를 AppDelegate에 직접 선언하고, 활성 터미널 뷰에 동일한 셀렉터로 포워딩합니다. SwiftTerm의 `MacTerminalView`가 `performFindPanelAction(_:)`을 구현하고 있어서 Cmd+F / Cmd+G / Cmd+Shift+G가 SwiftTerm 내장 검색바를 트리거합니다. 검색바에는 case-sensitive, regex, whole-word 옵션이 포함됩니다.

### TabViewController.swift (`@MainActor`)

탭 목록을 `[PaneManager]`로 관리. `contentArea`에 현재 탭의 `containerView`를 embed합니다.

**주요 흐름:**
```
newTab()
  └─ PaneManager 생성 → onLastPaneClosed / onActivePaneChanged 연결
  └─ NotificationCenter로 terminalTitleChanged 구독
  └─ switchTab(to: last)

switchTab(to:)
  └─ 이전 containerView removeFromSuperview
  └─ 새 containerView를 contentArea에 fill constraints로 embed
  └─ layoutSubtreeIfNeeded() → startShellIfNeeded() → makeFirstResponder()
```

**소켓 커맨드 메서드:** `tabList()`, `tabInfo()`, `renameTab(at:title:)`, `sessionList()`, `sessionInfo(index:)`, `execCommand(_:)`, `feedText(_:)`, `terminalState()`, `readScreen()`

### TabBarView.swift

NSView 기반 커스텀 탭바. TurmTheme 색상을 사용합니다.

- 각 탭: 제목 label + × 버튼, hover 시 배경색 전환
- 오른쪽 끝: + 버튼 (새 탭)
- `onSelectTab`, `onCloseTab`, `onNewTab` 클로저로 이벤트 전달
- `TabBarView.height = 36` (상수)

### PaneManager.swift (`@MainActor`)

단일 탭의 분할 창 트리를 관리합니다.

**핵심 설계:**
- `containerView` (NSView): TabViewController가 한 번만 embed, 이후 내부만 rebuild
- `root: SplitNode`: 현재 분할 상태의 N-ary 트리
- `activePane: TerminalViewController`: 현재 포커스된 터미널
- split/close 때마다 `rebuildViewHierarchy()` 호출 → fresh `EqualSplitView` 트리 생성

**`EqualSplitView` (private):**
`NSSplitView` + `NSSplitViewDelegate`를 결합한 내부 클래스.
`splitView(_:resizeSubviewsWithOldSize:)` delegate에서 첫 번째 호출 시 모든 subview를 균등 분배하고 `initialSizeSet = true`로 잠금. 이후 호출은 `adjustSubviews()`(기본 동작)에 위임해 사용자 드래그가 동작합니다.

`layout()`이 아닌 delegate를 쓰는 이유: NSSplitView는 `resizeSubviews`(→ delegate)로 subview frame을 확정한 뒤 `layout()`을 호출합니다. `layout()` 시점에는 이미 잘못된 frame이 커밋된 상태이므로 `setPosition`을 거기서 불러도 신뢰할 수 없습니다. delegate를 사용하면 NSSplitView가 "지금 subview frame 정해"라고 위임하는 바로 그 순간에 개입할 수 있습니다.

**포커스 감지:**
SwiftTerm의 `MacTerminalView.becomeFirstResponder`는 `public`이지만 `open`이 아니라서 외부 모듈에서 override 불가. `NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown)`으로 클릭 위치를 확인해 포커스 전환합니다.

**터미널 종료 연결:**
`wireTerminal(_:)`에서 `onProcessTerminated` 클로저 등록. 활성 pane이면 `closeActive()`, 비활성이면 트리에서 제거 후 rebuild.

### SplitNode.swift

N-ary 재귀 트리 enum:

```swift
indirect enum SplitNode {
    case leaf(TerminalViewController)
    case branch(SplitOrientation, [SplitNode])  // N개 자식 가능
}
```

**`splitting(_:with:orientation:)` — 항상 계층형 분할:**
focused leaf를 항상 새 2-child branch로 교체합니다. 같은 방향의 부모 branch에 flat하게 추가하지 않습니다.

```
[A] → split →
  branch(H, [leaf(A), leaf(B)])
  A=50%, B=50%

focus A → split →
  branch(H, [branch(H, [leaf(A), leaf(C)]), leaf(B)])
  A=25%, C=25%, B=50%  ← B 크기 변화 없음
```

**`removing(_:)` — collapse:**
제거 후 자식이 1개만 남으면 branch를 collapse해 단일 leaf로 승격.

### TerminalViewController.swift (`@MainActor`)

**`TurmTerminalView` (private subclass):**
SwiftTerm 버그 우회를 위한 래퍼. `installExitMonitor()`에서 별도 `DispatchSource.makeProcessSource`를 설치. 자세한 내용은 troubleshooting.md 참조.

**`startShellIfNeeded()`:**
Shell은 뷰가 계층에 추가되고 `layoutSubtreeIfNeeded()` 이후에만 시작. frame 없이 `startProcess`를 호출하면 SwiftTerm이 행/열 수를 0으로 계산합니다.

**환경변수:**
```swift
var env = ProcessInfo.processInfo.environment.map { "\($0.key)=\($0.value)" }
env.append("TERM=xterm-256color")
env.append("COLORTERM=truecolor")
env.append("TURM_SOCKET=/tmp/turm-\(pid).sock")
```
`startProcess(environment:)`에 배열을 넘기면 부모 환경이 완전히 교체되므로 반드시 현재 환경을 상속해야 합니다.

**소켓 커맨드 메서드:**
- `execCommand(_:)` — 명령어 + 줄바꿈 PTY 전송
- `feedText(_:)` — 원시 텍스트 PTY 전송
- `terminalState()` — cols/rows/cursor/title
- `readScreen()` — 현재 화면 텍스트 + 커서 위치
- `history(lines:)` — 스크롤백 N줄 (SwiftTerm 음수 row 인덱스 활용)
- `context(historyLines:)` — state + screen + history 합산
- `setCustomTitle(_:)` — 커스텀 탭 제목 (자동 업데이트 억제)

**`setTerminalTitle` delegate:**
```swift
nonisolated func setTerminalTitle(...) {
    Task { @MainActor in
        guard self.customTitle == nil else { return }  // 커스텀 제목이 있으면 무시
        self.currentTitle = title.isEmpty ? "Terminal" : title
        ...
    }
}
```

### Config.swift

`~/.config/turm/config.toml`을 직접 파싱.

**TOML 구조:**
```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrains Mono"
font_size = 13

[theme]
name = "catppuccin-mocha"
```

파싱 규칙: `[section]` 헤더로 섹션 구분, 따옴표/인라인 주석 제거.

**기본 폰트:** `"JetBrains Mono"` — macOS에 기본 설치된 NSFont family 이름. Nerd Font 변형(`JetBrainsMono Nerd Font Mono`)은 별도 설치 필요하므로 기본값으로 쓰지 않습니다.

### Theme.swift

`RGBColor(hex:)` — hex 문자열 → `(r, g, b: UInt8)` 변환.

SwiftTerm에 넘길 때 8비트 → 16비트 변환:
```swift
SwiftTerm.Color(red: UInt16(c.r) * 257, green: UInt16(c.g) * 257, blue: UInt16(c.b) * 257)
```
`* 257`(= `0x101`)로 곱하면 0 → 0, 255 → 65535로 정확히 매핑됩니다.

---

## 소켓 IPC

**아키텍처:**
```
turmctl ──Unix socket──► SocketServer (background thread)
                                │
                    DispatchQueue.main.sync
                                │
                       AppDelegate.handleCommand()
                                │
                    TabViewController / TerminalViewController
                                │
                       Response → socket thread → turmctl
```

프로토콜: turm-core `protocol.rs`와 동일한 newline-delimited JSON.

**지원 커맨드:**

| 커맨드 | 파라미터 | 동작 |
|---|---|---|
| `system.ping` | — | `{"status":"ok"}` |
| `terminal.exec` | `command` | 명령어 + 줄바꿈 PTY 전송 |
| `terminal.feed` | `text` | 원시 텍스트 PTY 전송 |
| `terminal.state` | — | cols/rows/cursor/title |
| `terminal.read` | — | 현재 화면 텍스트 |
| `terminal.history` | `lines` (기본 100) | 스크롤백 텍스트 |
| `terminal.context` | `history_lines` (기본 50) | state + screen + history |
| `tab.new` | — | 새 탭 생성 |
| `tab.close` | — | 활성 pane 닫기 |
| `tab.switch` | `index` | 탭 전환 |
| `tab.list` | — | 탭 목록 |
| `tab.info` | — | 탭 목록 + pane 수 |
| `tab.rename` | `index`, `title` | 탭 이름 변경 |
| `split.horizontal` | — | 좌우 분할 |
| `split.vertical` | — | 상하 분할 |
| `session.list` | — | `tab.list`와 동일 |
| `session.info` | `index` | 특정 탭의 상세 정보 |

---

## 알려진 주의사항

- **`fullSizeContentView` 금지**: 콘텐츠가 타이틀바 아래까지 확장되어 SwiftTerm이 행 수를 잘못 계산하고 커서 위치가 어긋납니다.
- **환경변수 상속 필수**: `startProcess(environment:)`에 배열을 넘기면 부모 환경 완전 교체. `TERM` 없이 쉘 실행 시 오류 발생.
- **`hostCurrentDirectoryUpdate` 구현 필수**: `LocalProcessTerminalViewDelegate`에 필수 메서드이므로 stub이라도 있어야 컴파일됩니다.
- **`startShellIfNeeded()` 타이밍**: `layoutSubtreeIfNeeded()` 이후에 호출해야 SwiftTerm이 올바른 cols/rows를 계산합니다.
- **`NSSplitView` subview layout**: NSSplitView의 직접 자식은 `translatesAutoresizingMaskIntoConstraints = true` + `autoresizingMask = [.width, .height]`를 써야 합니다. Auto Layout을 사용하면 NSSplitView와 충돌합니다.
- **SwiftTerm `processTerminated` 미호출 버그**: troubleshooting.md 참조.
