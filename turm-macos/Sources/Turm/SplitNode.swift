import AppKit

enum SplitOrientation {
    /// Vertical divider — panes side by side (Cmd+D)
    case horizontal
    /// Horizontal divider — panes stacked (Cmd+Shift+D)
    case vertical
}

/// Recursive split tree for a single tab.
/// Does NOT store NSSplitView/NSSplitViewController references — the view
/// hierarchy is rebuilt from scratch on every split/close operation.
indirect enum SplitNode {
    case leaf(TerminalViewController)
    case branch(SplitOrientation, SplitNode, SplitNode)

    // MARK: - Leaf enumeration

    func allLeaves() -> [TerminalViewController] {
        switch self {
        case let .leaf(vc): [vc]
        case let .branch(_, a, b): a.allLeaves() + b.allLeaves()
        }
    }

    // MARK: - Tree mutations

    /// Returns a new tree with `terminal`'s leaf replaced by `node`.
    func replacing(_ terminal: TerminalViewController, with node: SplitNode) -> SplitNode {
        switch self {
        case let .leaf(vc):
            vc === terminal ? node : self
        case let .branch(orientation, first, second):
            .branch(orientation, first.replacing(terminal, with: node), second.replacing(terminal, with: node))
        }
    }

    /// Returns a new tree with `terminal` removed, or nil if this was the only leaf.
    func removing(_ terminal: TerminalViewController) -> SplitNode? {
        switch self {
        case let .leaf(vc):
            return vc === terminal ? nil : self
        case let .branch(orientation, first, second):
            if case let .leaf(vc) = first, vc === terminal { return second }
            if case let .leaf(vc) = second, vc === terminal { return first }
            if let newFirst = first.removing(terminal) { return .branch(orientation, newFirst, second) }
            if let newSecond = second.removing(terminal) { return .branch(orientation, first, newSecond) }
            return self
        }
    }
}
