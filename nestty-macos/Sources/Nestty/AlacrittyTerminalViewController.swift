import AppKit
import CNesttyTerm
import Foundation

/// Phase 3.2 — `nestty-term` (alacritty_terminal-backed) pane with
/// CoreText cell rendering. Conforms to `NesttyPanel` so PaneManager
/// / SplitNode / socket commands treat it identically to
/// `TerminalViewController`. See
/// docs/macos-renderer-migration-plan.md for the staged scope.
///
/// What ships in this slice:
///
/// - PTY spawn (`NesttyTermFFI.Handle`) — already lands in 3.1
/// - CoreText cell draw — snapshot → row-by-row attributed strings
///   built from each run's borrowed utf8 + CTLine + CTLineDraw
/// - Periodic refresh (Timer at ~30 Hz) — Phase 3.6 will replace with
///   damage-tracked CADisplayLink
/// - Keyboard input — printable chars + the common control bytes
///   shells need to function (Return, Backspace, Tab, Esc, arrows)
///
/// Deferred:
///
/// - Cursor render (Phase 3.3)
/// - ANSI palette + inverse video (Phase 3.4)
/// - Image background + Zed-pattern materialize (Phase 3.5)
/// - Damage tracking + selection + IME + ligatures + automation
///   parity (Phases 3.6, 4, 5, 6, 7)
@MainActor
final class AlacrittyTerminalViewController: NSViewController, NesttyPanel {
    let panelID: String = UUID().uuidString
    private(set) var currentTitle: String = "Terminal (alacritty)"

    private let config: NesttyConfig
    private var theme: NesttyTheme
    private let initialCwd: String?
    private let initialInput: String?

    private var termHandle: NesttyTermFFI.Handle?
    private var renderView: AlacrittyRenderView?
    private var backgroundView: NSImageView?
    private var tintView: NSView?
    private var shellStarted = false

    init(config: NesttyConfig, theme: NesttyTheme, cwd: String? = nil, initialInput: String? = nil) {
        self.config = config
        self.theme = theme
        initialCwd = cwd
        self.initialInput = initialInput
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    /// Layered view hierarchy mirroring `TerminalViewController`:
    ///   container (focus-forwarding NSView)
    ///   ├─ backgroundView (NSImageView, hidden until image set)
    ///   ├─ tintView (NSView with dark overlay layer)
    ///   └─ renderView (AlacrittyRenderView, transparent layer when image active)
    ///
    /// External focus calls target `panel.view` (the container); the
    /// container forwards becomeFirstResponder to the render view so
    /// keystrokes still reach `keyDown` / NSTextInputClient.
    override func loadView() {
        let frame = NSRect(x: 0, y: 0, width: 1200, height: 800)
        let container = FocusForwardingView(frame: frame)
        container.wantsLayer = true

        let bg = NSImageView(frame: container.bounds)
        bg.autoresizingMask = [.width, .height]
        bg.imageScaling = .scaleAxesIndependently
        bg.wantsLayer = true
        bg.isHidden = true
        container.addSubview(bg)
        backgroundView = bg

        let tint = NSView(frame: container.bounds)
        tint.autoresizingMask = [.width, .height]
        tint.wantsLayer = true
        tint.isHidden = true
        container.addSubview(tint)
        tintView = tint

        let render = AlacrittyRenderView(
            theme: theme,
            font: resolveFont(family: config.fontFamily, size: CGFloat(config.fontSize)),
            transparentDefaultBg: config.transparentDefaultBg,
        )
        render.frame = container.bounds
        render.autoresizingMask = [.width, .height]
        container.addSubview(render)
        renderView = render
        container.focusTarget = render

        view = container

        // Apply background from config if set. Runs before viewDidAppear,
        // which is fine: NSImageView accepts an image even off-screen, and
        // we re-snap layer state in applyBackground itself.
        if let path = config.backgroundPath {
            applyBackground(path: path, tint: config.backgroundTint, opacity: config.backgroundOpacity)
        }
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        // Compute terminal grid size from view bounds + cell metrics so
        // the shell sees a winsize matching what we'll actually draw.
        guard let render = renderView else { return }
        let (cols, rows) = render.computeGrid()
        termHandle?.resize(cols: cols, rows: rows)
    }

    func startIfNeeded() {
        guard !shellStarted else { return }
        shellStarted = true
        let (cols, rows) = renderView?.computeGrid() ?? (80, 24)
        termHandle = NesttyTermFFI.Handle(
            cols: cols,
            rows: rows,
            shell: initialCwd != nil ? config.shell : nil,
            cwd: initialCwd,
        )
        if let initialInput {
            termHandle?.input(Array(initialInput.utf8))
        }
        renderView?.bind(handle: termHandle)
        // Target the render view explicitly. The container forwards too
        // (belt and braces for callers that already have a reference to
        // `panel.view`), but going direct skips the second hop.
        if let render = renderView {
            view.window?.makeFirstResponder(render)
        }
    }

    // MARK: - NesttyPanel — background

    /// Wire an image background + tint overlay. The render view's layer
    /// goes transparent so the image layer underneath composites
    /// through. `transparent_default_bg` config decides whether default
    /// cells fill opaquely on top (image hidden behind text area, cursor
    /// always visible) or stay transparent (image visible through blank
    /// cells, cursor visibility depends on accent vs image contrast).
    func applyBackground(path: String, tint: Double, opacity: Double) {
        guard let image = NSImage(contentsOfFile: path) else { return }
        backgroundView?.image = image
        backgroundView?.alphaValue = CGFloat(opacity)
        backgroundView?.isHidden = false
        tintView?.layer?.backgroundColor = NSColor.black.withAlphaComponent(CGFloat(tint)).cgColor
        tintView?.isHidden = opacity == 0
        renderView?.setImageBackgroundActive(true)
        renderView?.needsDisplay = true
    }

    func clearBackground() {
        backgroundView?.image = nil
        backgroundView?.isHidden = true
        tintView?.isHidden = true
        renderView?.setImageBackgroundActive(false)
        renderView?.needsDisplay = true
    }

    func setTint(_ alpha: Double) {
        tintView?.layer?.backgroundColor = NSColor.black.withAlphaComponent(CGFloat(alpha)).cgColor
    }

    // MARK: - Font

    /// Mirrors `TerminalViewController.resolveFont` — PostScript name
    /// → family lookup → case-insensitive fallback → monospaced
    /// system. Trimmed to the cases we need for the alacritty path.
    private func resolveFont(family: String, size: CGFloat) -> NSFont {
        if let font = NSFont(name: family, size: size) { return font }
        let manager = NSFontManager.shared
        if let font = manager.font(withFamily: family, traits: [], weight: 5, size: size) {
            return font
        }
        let lower = family.lowercased()
        for fam in manager.availableFontFamilies where fam.lowercased() == lower {
            if let font = manager.font(withFamily: fam, traits: [], weight: 5, size: size) {
                return font
            }
        }
        return .monospacedSystemFont(ofSize: size, weight: .regular)
    }
}

// MARK: - Focus-forwarding container

/// Container view whose only job is to bounce first-responder requests
/// to its embedded render view. The PaneManager / TabViewController
/// codepaths target `panel.view` (the root NSView) when activating a
/// pane; without this redirect, `keyDown` and NSTextInputClient on the
/// render subview never fire because the container itself doesn't
/// override either.
@MainActor
private final class FocusForwardingView: NSView {
    weak var focusTarget: NSView?

    override var acceptsFirstResponder: Bool {
        focusTarget?.acceptsFirstResponder ?? false
    }

    override func becomeFirstResponder() -> Bool {
        // Re-entering NSWindow.makeFirstResponder inside becomeFirstResponder
        // is fragile (the window is mid-dispatch). Accept becoming first
        // responder here, then defer the swap to the next runloop turn
        // so the window's responder-transition completes cleanly before
        // we re-target. Keystrokes can't arrive between these ticks.
        guard let target = focusTarget else { return false }
        DispatchQueue.main.async { [weak self] in
            _ = self?.window?.makeFirstResponder(target)
        }
        return true
    }
}

// MARK: - Render view

/// Custom NSView that draws the terminal grid via CoreText. Snapshots
/// are taken under the `nestty-term` handle's `FairMutex`; the lock is
/// dropped before `setNeedsDisplay` so AppKit's redraw doesn't block
/// the PTY reader thread.
///
/// Coordinate system is **flipped** (origin top-left, y down) so row 0
/// renders at the top of the view — matching the terminal convention
/// and keeping cell math straightforward.
@MainActor
private final class AlacrittyRenderView: NSView, @preconcurrency NSTextInputClient {
    private let theme: NesttyTheme
    private var font: NSFont
    private var boldFont: NSFont
    private(set) var cellWidth: CGFloat = 0
    private(set) var cellHeight: CGFloat = 0
    private var ascent: CGFloat = 0

    /// Cached CGColor for the 16-color ANSI palette + xterm 256
    /// extension. Indices 0-15 from `theme.palette` (so theme changes
    /// reflect the right color); 16-231 from the 6×6×6 cube; 232-255
    /// from the grayscale ramp.
    private let paletteCache: [CGColor]

    private weak var termHandle: NesttyTermFFI.Handle?
    /// nonisolated(unsafe) so deinit (Swift 6 nonisolated) can
    /// invalidate the timer without crossing the main-actor barrier.
    /// Same RAII pattern used by NesttyTermFFI.Handle/Snapshot.
    private nonisolated(unsafe) var refreshTimer: Timer?

    /// Cached snapshot for the most recent paint. Phase 3.6 will
    /// switch to damage-tracked partial repaints; for now the whole
    /// view repaints when the timer fires.
    private var snapshotCache: NesttyTermFFI.Snapshot?

    /// User opt-in: when true AND an image background is active, default
    /// (sentinel-zero) cells render without a bg fill so the image shows
    /// through. Independent of the controller's bg state because the
    /// flag is set at init from the live config; the
    /// `imageBackgroundActive` runtime flag (set/cleared as the user
    /// applies or clears the background) AND-gates the actual behavior
    /// — no image, no transparency, regardless of the user pref.
    private let transparentDefaultBg: Bool
    private var imageBackgroundActive = false

    init(theme: NesttyTheme, font: NSFont, transparentDefaultBg: Bool) {
        self.theme = theme
        self.font = font
        boldFont = Self.deriveBold(from: font)
        paletteCache = Self.buildPalette(theme: theme)
        self.transparentDefaultBg = transparentDefaultBg
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = theme.background.nsColor.cgColor
        recomputeCellMetrics()
        startRefreshTimer()
    }

    /// Called by the controller when `applyBackground` / `clearBackground`
    /// flips the layered-view state. Toggles the layer-bg clear/opaque
    /// AND the bounds-fill skip — both are needed: layer-bg covers the
    /// image even without per-cell draw, and the bounds fill would
    /// re-cover it inside `draw(_:)`.
    func setImageBackgroundActive(_ active: Bool) {
        imageBackgroundActive = active
        layer?.backgroundColor = active
            ? NSColor.clear.cgColor
            : theme.background.nsColor.cgColor
        needsDisplay = true
    }

    private static func deriveBold(from regular: NSFont) -> NSFont {
        let mgr = NSFontManager.shared
        if let bold = mgr.convert(regular, toHaveTrait: .boldFontMask) as NSFont? {
            return bold
        }
        return regular
    }

    /// 256-color ANSI table, computed once at view init. Indices 0-15
    /// follow theme.palette so a theme change re-derives the right
    /// brand colors; 16-231 are the canonical xterm 6×6×6 cube;
    /// 232-255 are the 24-step grayscale ramp.
    private static func buildPalette(theme: NesttyTheme) -> [CGColor] {
        var out: [CGColor] = []
        out.reserveCapacity(256)
        for c in theme.palette {
            out.append(c.nsColor.cgColor)
        }
        // Defensive padding if a theme ships fewer than 16 palette
        // entries — black for the missing slots so a stray index
        // doesn't crash.
        while out.count < 16 {
            out.append(CGColor(red: 0, green: 0, blue: 0, alpha: 1))
        }
        // 6×6×6 RGB cube (216 colors).
        let cubeLevels: [CGFloat] = [0, 95, 135, 175, 215, 255].map { $0 / 255.0 }
        for r in 0 ..< 6 {
            for g in 0 ..< 6 {
                for b in 0 ..< 6 {
                    out.append(CGColor(red: cubeLevels[r], green: cubeLevels[g], blue: cubeLevels[b], alpha: 1))
                }
            }
        }
        // 24-step grayscale.
        for i in 0 ..< 24 {
            let v = CGFloat(8 + i * 10) / 255.0
            out.append(CGColor(red: v, green: v, blue: v, alpha: 1))
        }
        return out
    }

    /// Decode the fg/bg encoding from `nestty-term::color_to_rgba`.
    /// High byte is a tag: 0x00=default, 0x01=indexed (low byte holds
    /// the index), 0xFF=direct RGB in the low 24 bits. Tagged because
    /// the old "alpha byte = 0 means indexed" trick collided with RGB
    /// colors that have R=0 (skyblue, pure green) — those silently
    /// fell into the indexed path and rendered as grayscale.
    private func resolveColor(_ packed: UInt32, defaultColor: CGColor) -> CGColor {
        let tag = (packed >> 24) & 0xFF
        switch tag {
        case 0x00:
            return defaultColor
        case 0x01:
            let idx = Int(packed & 0xFF)
            return idx < paletteCache.count ? paletteCache[idx] : defaultColor
        case 0xFF:
            let r = CGFloat((packed >> 16) & 0xFF) / 255.0
            let g = CGFloat((packed >> 8) & 0xFF) / 255.0
            let b = CGFloat(packed & 0xFF) / 255.0
            return CGColor(red: r, green: g, blue: b, alpha: 1.0)
        default:
            return defaultColor
        }
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    deinit {
        refreshTimer?.invalidate()
    }

    override var isFlipped: Bool {
        true
    }

    override var acceptsFirstResponder: Bool {
        true
    }

    func bind(handle: NesttyTermFFI.Handle?) {
        termHandle = handle
    }

    func computeGrid() -> (cols: UInt16, rows: UInt16) {
        let w = max(1, Int(bounds.width / cellWidth))
        let h = max(1, Int(bounds.height / cellHeight))
        return (UInt16(min(w, Int(UInt16.max))), UInt16(min(h, Int(UInt16.max))))
    }

    private func recomputeCellMetrics() {
        // Monospaced advance: measure the wide-but-canonical "M".
        // Falls back to font.maximumAdvancement if measurement fails
        // (shouldn't on a real monospaced face).
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let m = NSAttributedString(string: "M", attributes: attrs)
        cellWidth = ceil(m.size().width)
        if cellWidth <= 0 { cellWidth = ceil(font.maximumAdvancement.width) }
        ascent = ceil(font.ascender)
        let descent = ceil(abs(font.descender))
        let leading = ceil(font.leading)
        cellHeight = ascent + descent + leading
        if cellHeight <= 0 { cellHeight = 16 }
    }

    private func startRefreshTimer() {
        refreshTimer?.invalidate()
        // ~30 Hz. CADisplayLink + damage tracking is Phase 3.6; for
        // 3.2 a Timer is sufficient and avoids the display-link
        // run-loop integration tax up front.
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            // Timer fires on the main runloop (scheduledTimer's
            // default). Assume-isolated lets us call the @MainActor
            // tick() without a hop, matching the actual thread.
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    private func tick() {
        guard let handle = termHandle else { return }
        // Take a fresh snapshot and trigger a redraw. The snapshot is
        // a copy of the grid (Rust-side `Box`); holding it across the
        // draw is cheap.
        let snap = handle.snapshot()
        snapshotCache = snap
        needsDisplay = true
    }

    override func draw(_: NSRect) {
        guard let snap = snapshotCache,
              let ctx = NSGraphicsContext.current?.cgContext
        else { return }

        // Fill the bounds with theme background unless the user opted
        // into transparent default cells AND a background image is
        // active. In that case the image layer underneath shows through
        // blank cells; cells with explicit ANSI bg or reverse-video
        // still materialize opaque in `drawRow` (Zed pattern), so
        // reverse-video stays legible against the image.
        if !(transparentDefaultBg && imageBackgroundActive) {
            ctx.setFillColor(theme.background.nsColor.cgColor)
            ctx.fill(bounds)
        }

        // Cursor first (under the text), so block-style cursor
        // shows its character on top via the text loop. Hidden
        // (style=0) — short-circuit. Phase 3.4 will overlay the
        // character with `caretTextColor` when the cursor cell is a
        // block; for now the foreground glyph just draws over the
        // accent fill.
        drawCursor(snap.cursor)

        // CTLineDraw uses CoreGraphics-native y-up glyph orientation.
        // Our view is `isFlipped = true` (so row 0 is at the top
        // visually) — without this textMatrix flip the glyphs render
        // upside-down + mirrored against the flipped CTM. Save/restore
        // the prior state so we don't leak the flip into non-text
        // drawing later.
        ctx.saveGState()
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        defer { ctx.restoreGState() }

        let snapRows = snap.rows
        for row in 0 ..< snapRows {
            let runs = snap.rowRuns(row)
            let utf8 = snap.rowUtf8(row)
            guard runs.count > 0, utf8.count > 0 else { continue }
            drawRow(row: row, runs: runs, utf8: utf8, ctx: ctx)
        }
    }

    /// Cursor render. Style 0 = hidden (skip). Block (1) fills the
    /// whole cell. Beam (2) is a 2-px vertical bar at the cell's
    /// leading edge. Underline (3) is a 2-px horizontal bar at the
    /// cell's bottom. When the window isn't key (e.g. user switched
    /// apps), block style draws as a hollow outline so the user can
    /// tell the terminal won't receive input — Terminal.app + iTerm2
    /// do the same.
    private func drawCursor(_ cursor: NesttyCursor) {
        guard cursor.style != 0,
              let ctx = NSGraphicsContext.current?.cgContext
        else { return }
        let x = CGFloat(cursor.col) * cellWidth
        let y = CGFloat(cursor.row) * cellHeight
        let cell = CGRect(x: x, y: y, width: cellWidth, height: cellHeight)
        let isKey = window?.isKeyWindow ?? false
        let color = theme.accent.nsColor.cgColor

        switch cursor.style {
        case 1: // block
            if isKey {
                ctx.setFillColor(color)
                ctx.fill(cell)
            } else {
                ctx.setStrokeColor(color)
                ctx.setLineWidth(1)
                // Stroke is centered on the path; inset by half so
                // it stays inside the cell rect.
                ctx.stroke(cell.insetBy(dx: 0.5, dy: 0.5))
            }
        case 2: // beam (bar)
            let barWidth: CGFloat = 2
            let rect = CGRect(x: x, y: y, width: barWidth, height: cellHeight)
            ctx.setFillColor(color)
            ctx.fill(rect)
        case 3: // underline
            let barHeight: CGFloat = 2
            let rect = CGRect(x: x, y: y + cellHeight - barHeight, width: cellWidth, height: barHeight)
            ctx.setFillColor(color)
            ctx.fill(rect)
        default:
            break
        }
    }

    private func drawRow(
        row: UInt16,
        runs: UnsafeBufferPointer<NesttyRun>,
        utf8: UnsafeBufferPointer<UInt8>,
        ctx: CGContext,
    ) {
        // Baseline in flipped coords: top of row + ascent.
        let baselineY = CGFloat(row) * cellHeight + ascent
        let defaultFg = theme.foreground.nsColor.cgColor
        let defaultBg = theme.background.nsColor.cgColor

        // Flag bits mirror nestty_term::flags (see
        // nestty-term/src/lib.rs). Kept as Swift constants to avoid
        // a third source of truth.
        let flagBold: UInt16 = 1 << 0
        let flagInverse: UInt16 = 1 << 3
        let flagDim: UInt16 = 1 << 4

        let transparentMode = transparentDefaultBg && imageBackgroundActive

        for i in 0 ..< runs.count {
            let run = runs[i]

            // Provenance — was this cell's bg from the default sentinel
            // (`run.bg_rgba == 0`)? We need to know BEFORE resolving so
            // we can decide whether transparent mode applies. Equality
            // check on the resolved color is not enough: an explicit
            // ANSI bg that happens to equal theme.background should
            // still paint (it's a real intent), and a real default cell
            // should NOT paint in transparent mode even though its
            // resolved color matches theme.bg.
            let bgIsDefault = run.bg_rgba == 0
            let isInverse = run.flags & flagInverse != 0

            // Resolve colors then apply inverse swap. Default-bg
            // materializes to theme.background BEFORE the swap (Zed
            // pattern from §Phase 3 in the plan — reverse-video over
            // transparent bg would render invisibly without it).
            var fg = resolveColor(run.fg_rgba, defaultColor: defaultFg)
            var bg = resolveColor(run.bg_rgba, defaultColor: defaultBg)
            if isInverse {
                swap(&fg, &bg)
            }
            // Dim → reduce fg alpha. ANSI spec is intentionally vague
            // here; ~65% is the conventional value across emulators.
            if run.flags & flagDim != 0, let dimmed = fg.copy(alpha: 0.65) {
                fg = dimmed
            }

            let x = CGFloat(run.start_col) * cellWidth
            let cellsWide = CGFloat(run.end_col - run.start_col)
            let cellRect = CGRect(x: x, y: CGFloat(row) * cellHeight,
                                  width: cellsWide * cellWidth, height: cellHeight)

            // Per-cell bg fill — overrides the global bounds fill.
            //   Opaque mode: skip when the resolved bg equals theme.bg
            //     (the bounds fill already covered it).
            //   Transparent mode: skip only when the cell came from the
            //     default sentinel AND is not inverse — those are the
            //     only cells we let the image bleed through. Inverse +
            //     default-bg is opaque theme.fg after swap and must
            //     still paint.
            let skipFill = transparentMode
                ? (bgIsDefault && !isInverse)
                : cgColorsApproxEqual(bg, defaultBg)
            if !skipFill {
                ctx.setFillColor(bg)
                ctx.fill(cellRect)
            }

            // Text. Empty/whitespace skipped to save a CTLine alloc.
            let len = Int(run.utf8_len)
            let offset = Int(run.utf8_offset)
            guard offset + len <= utf8.count else { continue }
            guard
                let str = String(bytes: UnsafeBufferPointer(rebasing: utf8[offset ..< offset + len]), encoding: .utf8),
                !str.isEmpty
            else { continue }

            let runFont = (run.flags & flagBold != 0) ? boldFont : font
            var attrs: [NSAttributedString.Key: Any] = [
                .font: runFont,
                .foregroundColor: NSColor(cgColor: fg) ?? .white,
            ]
            // underline_style: 0=none, others=show single (Phase 3.7
            // will distinguish double/curly/dotted/dashed).
            if run.underline_style != 0 {
                attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
                let ulColor = run.underline_color_rgba == 0
                    ? fg
                    : resolveColor(run.underline_color_rgba, defaultColor: fg)
                attrs[.underlineColor] = NSColor(cgColor: ulColor) ?? .white
            }
            let attr = NSAttributedString(string: str, attributes: attrs)
            let line = CTLineCreateWithAttributedString(attr)
            ctx.textPosition = CGPoint(x: x, y: baselineY)
            CTLineDraw(line, ctx)
        }
    }

    /// Cheap component-wise equality for the "is this cell's bg the
    /// same as the bounds fill we already did" early-out. Falls back
    /// to ObjectIdentifier when components aren't comparable (mixed
    /// color spaces).
    private func cgColorsApproxEqual(_ a: CGColor, _ b: CGColor) -> Bool {
        guard let ac = a.components, let bc = b.components, ac.count == bc.count else { return false }
        for (x, y) in zip(ac, bc) where abs(x - y) > 0.001 {
            return false
        }
        return true
    }

    // MARK: - Keyboard

    /// Route keyDown through `interpretKeyEvents` so the system IME
    /// (Korean 2-Set, Japanese, …) sees the keystrokes. Without this,
    /// IME-active keystrokes don't deliver committed text back via
    /// `event.characters` and Korean/Japanese input silently drops.
    /// Preedit-text rendering during composition is still Phase 6;
    /// this slice just lets COMMITTED IME text flow into the PTY.
    ///
    /// Ctrl-letter combinations bypass IME entirely because shells +
    /// TUIs rely on them as raw control bytes (Ctrl+C = 0x03 → SIGINT,
    /// Ctrl+D = 0x04 → EOF). Cmd-modified keys go to the responder
    /// chain (menu shortcuts, clipboard) by calling super.
    override func keyDown(with event: NSEvent) {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if mods.contains(.command) {
            super.keyDown(with: event)
            return
        }
        if mods == .control, let bytes = controlBytes(for: event) {
            termHandle?.input(bytes)
            return
        }
        interpretKeyEvents([event])
    }

    /// Map Ctrl+letter / Ctrl+@ / Ctrl+[ / Ctrl+\ / Ctrl+] / Ctrl+^
    /// / Ctrl+_ / Ctrl+Space to their canonical control bytes
    /// (0x00–0x1f, 0x7f). Returns nil for combinations not in the
    /// standard ASCII control set so the responder chain can handle
    /// them.
    private func controlBytes(for event: NSEvent) -> [UInt8]? {
        guard
            let chars = event.charactersIgnoringModifiers?.lowercased(),
            let scalar = chars.unicodeScalars.first
        else { return nil }
        let v = scalar.value
        // a-z → 0x01-0x1a
        if (0x61 ... 0x7A).contains(v) { return [UInt8(v - 0x60)] }
        switch v {
        case 0x20: return [0x00] // Ctrl+Space → NUL
        case 0x40: return [0x00] // Ctrl+@ → NUL
        case 0x5B: return [0x1B] // Ctrl+[ → ESC
        case 0x5C: return [0x1C] // Ctrl+\
        case 0x5D: return [0x1D] // Ctrl+]
        case 0x5E: return [0x1E] // Ctrl+^
        case 0x5F: return [0x1F] // Ctrl+_
        case 0x3F: return [0x7F] // Ctrl+? → DEL
        default: return nil
        }
    }

    // NSTextInputClient — IME routes commits through `insertText` and
    // special keys through `doCommand`. Phase 6 will flesh out the
    // marked-text path so preedit characters render in-cell during
    // composition.

    func insertText(_ string: Any, replacementRange _: NSRange) {
        let text: String
        if let s = string as? String { text = s }
        else if let a = string as? NSAttributedString { text = a.string }
        else { return }
        guard !text.isEmpty else { return }
        termHandle?.input(Array(text.utf8))
    }

    override func doCommand(by selector: Selector) {
        if let bytes = commandBytes(for: selector) {
            termHandle?.input(bytes)
        }
        // Unmapped selectors fall on the floor — better than calling
        // super which would try to interpret them as text editing on
        // a view that has no document model.
    }

    /// Selectors AppKit's text-input system synthesizes for keys that
    /// aren't plain printable characters. Mapped to the byte sequences
    /// a VT100-ish terminal expects.
    private func commandBytes(for selector: Selector) -> [UInt8]? {
        switch selector {
        case #selector(NSStandardKeyBindingResponding.insertNewline(_:)):
            [0x0D]
        case #selector(NSStandardKeyBindingResponding.insertTab(_:)):
            [0x09]
        case #selector(NSStandardKeyBindingResponding.deleteBackward(_:)):
            [0x7F]
        case #selector(NSStandardKeyBindingResponding.deleteForward(_:)):
            [0x1B, 0x5B, 0x33, 0x7E] // ESC [ 3 ~
        case #selector(NSStandardKeyBindingResponding.cancelOperation(_:)):
            [0x1B]
        case #selector(NSStandardKeyBindingResponding.moveLeft(_:)):
            [0x1B, 0x5B, 0x44]
        case #selector(NSStandardKeyBindingResponding.moveRight(_:)):
            [0x1B, 0x5B, 0x43]
        case #selector(NSStandardKeyBindingResponding.moveUp(_:)):
            [0x1B, 0x5B, 0x41]
        case #selector(NSStandardKeyBindingResponding.moveDown(_:)):
            [0x1B, 0x5B, 0x42]
        default:
            nil
        }
    }

    // Stubs — Phase 6 implements preedit rendering and candidate
    // window positioning via these methods.

    func setMarkedText(_: Any, selectedRange _: NSRange, replacementRange _: NSRange) {}
    func unmarkText() {}
    func selectedRange() -> NSRange {
        NSRange(location: NSNotFound, length: 0)
    }

    func markedRange() -> NSRange {
        NSRange(location: NSNotFound, length: 0)
    }

    func hasMarkedText() -> Bool {
        false
    }

    func attributedSubstring(forProposedRange _: NSRange, actualRange _: NSRangePointer?) -> NSAttributedString? {
        nil
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        []
    }

    func firstRect(forCharacterRange _: NSRange, actualRange _: NSRangePointer?) -> NSRect {
        .zero
    }

    func characterIndex(for _: NSPoint) -> Int {
        NSNotFound
    }
}
