import AppKit

/// Manages the split-pane tree for a single tab.
/// TabViewController embeds `containerView` once; PaneManager swaps its contents on splits/closes.
@MainActor
final class PaneManager {
    private let config: TurmConfig
    private let theme: TurmTheme

    private(set) var root: SplitNode
    private(set) var activePane: TerminalViewController

    /// Stable container — TabViewController pins this to contentArea once and never re-embeds.
    let containerView: NSView

    var onLastPaneClosed: (() -> Void)?
    var onActivePaneChanged: (() -> Void)?

    private nonisolated(unsafe) var clickMonitor: Any?

    // MARK: - Init

    init(config: TurmConfig, theme: TurmTheme) {
        self.config = config
        self.theme = theme

        let termVC = TerminalViewController(config: config, theme: theme)
        root = .leaf(termVC)
        activePane = termVC

        containerView = NSView()
        containerView.translatesAutoresizingMaskIntoConstraints = false

        // Finish setup after all stored properties are initialized
        wireTerminal(termVC)
        embedRoot()
        installClickMonitor()
    }

    deinit {
        if let m = clickMonitor { NSEvent.removeMonitor(m) }
    }

    // MARK: - Public API

    func splitActive(orientation: SplitOrientation) {
        let newTermVC = TerminalViewController(config: config, theme: theme)
        wireTerminal(newTermVC)

        let svc = makeSplitVC(orientation: orientation, first: activePane, second: newTermVC)
        let newBranch = SplitNode.branch(svc, orientation, .leaf(activePane), .leaf(newTermVC))
        root = root.replacing(activePane, with: newBranch)

        embedRoot()

        // Equal split after layout
        let splitView = svc.splitView
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.03) {
            let size = splitView.isVertical ? splitView.frame.width : splitView.frame.height
            splitView.setPosition(size / 2, ofDividerAt: 0)
        }

        setActive(newTermVC)
        newTermVC.startShellIfNeeded()
        newTermVC.view.window?.makeFirstResponder(newTermVC.view)
    }

    func closeActive() {
        let closing = activePane
        let newRoot = root.removing(closing)

        // Teardown the closing terminal
        closing.view.removeFromSuperview()
        closing.removeFromParent()

        guard let newRoot else {
            onLastPaneClosed?()
            return
        }

        root = newRoot

        // Focus the first leaf of the remaining tree
        let next = root.allLeaves().first!
        embedRoot()
        setActive(next)
        next.view.window?.makeFirstResponder(next.view)
    }

    func setActive(_ terminal: TerminalViewController) {
        activePane = terminal
        onActivePaneChanged?()
    }

    func allTerminals() -> [TerminalViewController] {
        root.allLeaves()
    }

    // MARK: - Private

    private func installClickMonitor() {
        clickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            guard let self else { return event }
            let leaves = root.allLeaves()
            guard leaves.count > 1 else { return event }
            for termVC in leaves {
                let view = termVC.view
                let locationInView = view.convert(event.locationInWindow, from: nil)
                if view.bounds.contains(locationInView) {
                    setActive(termVC)
                    break
                }
            }
            return event
        }
    }

    private func wireTerminal(_ termVC: TerminalViewController) {
        termVC.onProcessTerminated = { [weak self, weak termVC] in
            guard let self, let termVC else { return }
            if termVC === activePane {
                closeActive()
            } else {
                // Non-active pane exited: remove it silently
                let newRoot = root.removing(termVC)
                termVC.view.removeFromSuperview()
                termVC.removeFromParent()
                if let newRoot {
                    root = newRoot
                    embedRoot()
                } else {
                    onLastPaneClosed?()
                }
            }
        }
    }

    private func embedRoot() {
        // Remove existing subview
        containerView.subviews.forEach { $0.removeFromSuperview() }

        let rootView = root.rootViewController.view
        rootView.translatesAutoresizingMaskIntoConstraints = false
        containerView.addSubview(rootView)
        NSLayoutConstraint.activate([
            rootView.topAnchor.constraint(equalTo: containerView.topAnchor),
            rootView.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
            rootView.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
            rootView.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
        ])
    }
}
