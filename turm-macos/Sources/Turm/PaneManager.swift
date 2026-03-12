import AppKit

/// Manages the split-pane tree for a single tab.
/// TabViewController embeds `containerView` once; PaneManager rebuilds its
/// contents on every split/close using fresh NSSplitView instances.
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
    /// Tracks the fill constraints added to containerView so they can be
    /// deactivated before the next rebuild.
    private var rootConstraints: [NSLayoutConstraint] = []

    // MARK: - Init

    init(config: TurmConfig, theme: TurmTheme) {
        self.config = config
        self.theme = theme

        let termVC = TerminalViewController(config: config, theme: theme)
        root = .leaf(termVC)
        activePane = termVC

        containerView = NSView()
        containerView.translatesAutoresizingMaskIntoConstraints = false

        wireTerminal(termVC)
        rebuildViewHierarchy()
        installClickMonitor()
    }

    deinit {
        if let m = clickMonitor { NSEvent.removeMonitor(m) }
    }

    // MARK: - Public API

    func splitActive(orientation: SplitOrientation) {
        let newTermVC = TerminalViewController(config: config, theme: theme)
        wireTerminal(newTermVC)

        let newBranch = SplitNode.branch(orientation, .leaf(activePane), .leaf(newTermVC))
        root = root.replacing(activePane, with: newBranch)

        rebuildViewHierarchy()

        setActive(newTermVC)
        newTermVC.startShellIfNeeded()
        newTermVC.view.window?.makeFirstResponder(newTermVC.view)
    }

    func closeActive() {
        let closing = activePane
        guard let newRoot = root.removing(closing) else {
            closing.view.removeFromSuperview()
            closing.removeFromParent()
            onLastPaneClosed?()
            return
        }

        root = newRoot
        closing.view.removeFromSuperview()
        closing.removeFromParent()
        rebuildViewHierarchy()

        let next = root.allLeaves().first!
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

    func setCustomTitle(_ title: String) {
        activePane.setCustomTitle(title)
    }

    // MARK: - View Hierarchy

    /// Rebuilds the entire view hierarchy from the SplitNode tree.
    /// This is called on every split/close, creating fresh NSSplitViews each time.
    private func rebuildViewHierarchy() {
        NSLayoutConstraint.deactivate(rootConstraints)
        rootConstraints = []
        containerView.subviews.forEach { $0.removeFromSuperview() }

        var splitViews: [NSSplitView] = []
        let rootView = buildView(from: root, splitViews: &splitViews)
        // Root view uses Auto Layout to fill containerView
        rootView.translatesAutoresizingMaskIntoConstraints = false
        containerView.addSubview(rootView)

        let constraints = [
            rootView.topAnchor.constraint(equalTo: containerView.topAnchor),
            rootView.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
            rootView.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
            rootView.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
        ]
        NSLayoutConstraint.activate(constraints)
        rootConstraints = constraints

        // Set equal 50/50 split after layout resolves
        if !splitViews.isEmpty {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                for sv in splitViews {
                    let size = sv.isVertical ? sv.frame.width : sv.frame.height
                    guard size > 0 else { continue }
                    sv.setPosition(size / 2, ofDividerAt: 0)
                }
            }
        }
    }

    /// Recursively builds the view tree. NSSplitView manages subview sizing,
    /// so direct children use translatesAutoresizingMaskIntoConstraints = true.
    private func buildView(from node: SplitNode, splitViews: inout [NSSplitView]) -> NSView {
        switch node {
        case let .leaf(vc):
            vc.view.translatesAutoresizingMaskIntoConstraints = true
            vc.view.autoresizingMask = [.width, .height]
            return vc.view

        case let .branch(orientation, first, second):
            let sv = NSSplitView()
            sv.isVertical = (orientation == .horizontal)
            sv.dividerStyle = .thin
            sv.addSubview(buildView(from: first, splitViews: &splitViews))
            sv.addSubview(buildView(from: second, splitViews: &splitViews))
            splitViews.append(sv)
            return sv
        }
    }

    // MARK: - Focus Monitor

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

    // MARK: - Terminal Wiring

    private func wireTerminal(_ termVC: TerminalViewController) {
        termVC.onProcessTerminated = { [weak self, weak termVC] in
            guard let self, let termVC else { return }
            if termVC === activePane {
                closeActive()
            } else {
                guard let newRoot = root.removing(termVC) else {
                    onLastPaneClosed?(); return
                }
                termVC.view.removeFromSuperview()
                termVC.removeFromParent()
                root = newRoot
                rebuildViewHierarchy()
            }
        }
    }
}
