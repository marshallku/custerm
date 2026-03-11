import AppKit

enum SplitOrientation {
    /// Vertical divider — panes side by side (Cmd+D)
    case horizontal
    /// Horizontal divider — panes stacked (Cmd+Shift+D)
    case vertical
}

/// Recursive split tree for a single tab.
/// Each leaf is a TerminalViewController; each branch is an NSSplitViewController.
indirect enum SplitNode {
    case leaf(TerminalViewController)
    case branch(NSSplitViewController, SplitOrientation, SplitNode, SplitNode)

    // MARK: - Root view controller

    var rootViewController: NSViewController {
        switch self {
        case let .leaf(vc): vc
        case let .branch(svc, _, _, _): svc
        }
    }

    // MARK: - Leaf enumeration

    func allLeaves() -> [TerminalViewController] {
        switch self {
        case let .leaf(vc):
            [vc]
        case let .branch(_, _, a, b):
            a.allLeaves() + b.allLeaves()
        }
    }

    // MARK: - Tree mutations

    /// Returns a new tree with `terminal`'s leaf replaced by `node`.
    func replacing(_ terminal: TerminalViewController, with node: SplitNode) -> SplitNode {
        switch self {
        case let .leaf(vc):
            return vc === terminal ? node : self
        case let .branch(svc, orientation, first, second):
            let newFirst = first.replacing(terminal, with: node)
            let newSecond = second.replacing(terminal, with: node)
            return .branch(svc, orientation, newFirst, newSecond)
        }
    }

    /// Returns a new tree with `terminal` removed, or nil if this was the only leaf.
    /// The sibling of the removed leaf promotes up to replace the parent branch.
    func removing(_ terminal: TerminalViewController) -> SplitNode? {
        switch self {
        case let .leaf(vc):
            return vc === terminal ? nil : self
        case let .branch(_, _, first, second):
            if case let .leaf(vc) = first, vc === terminal {
                return second
            }
            if case let .leaf(vc) = second, vc === terminal {
                return first
            }
            if let newFirst = first.removing(terminal) {
                return rebranch(self, first: newFirst, second: second)
            }
            if let newSecond = second.removing(terminal) {
                return rebranch(self, first: first, second: newSecond)
            }
            return self
        }
    }
}

// MARK: - Helpers

private func rebranch(_ original: SplitNode, first: SplitNode, second: SplitNode) -> SplitNode {
    guard case let .branch(svc, orientation, _, _) = original else { return original }
    return .branch(svc, orientation, first, second)
}

// MARK: - NSSplitViewController factory

@MainActor
func makeSplitVC(
    orientation: SplitOrientation,
    first: NSViewController,
    second: NSViewController,
) -> NSSplitViewController {
    let svc = NSSplitViewController()
    // isVertical=true → vertical divider → panes side by side (horizontal split)
    svc.splitView.isVertical = (orientation == .horizontal)
    svc.splitView.dividerStyle = .thin

    let item1 = NSSplitViewItem(viewController: first)
    item1.minimumThickness = 80
    let item2 = NSSplitViewItem(viewController: second)
    item2.minimumThickness = 80

    svc.addSplitViewItem(item1)
    svc.addSplitViewItem(item2)
    return svc
}
