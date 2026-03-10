import AppKit

@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    var terminalVC: TerminalViewController?
    private let socketServer = SocketServer()

    func applicationDidFinishLaunching(_: Notification) {
        let config = TurmConfig.load()
        let theme = TurmTheme.byName(config.themeName) ?? .catppuccinMocha

        setupMenuBar()

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false,
        )
        window.title = "turm"
        window.center()
        window.backgroundColor = theme.background.nsColor

        let termVC = TerminalViewController(config: config, theme: theme)
        window.contentViewController = termVC

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        self.window = window
        terminalVC = termVC

        startSocketServer()
    }

    func applicationWillTerminate(_: Notification) {
        socketServer.stop()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        true
    }

    // MARK: - Socket Server

    private func startSocketServer() {
        socketServer.commandHandler = { [weak self] method, params in
            self?.handleCommand(method: method, params: params)
        }
        socketServer.start()
    }

    private func handleCommand(method: String, params: [String: Any]) -> Any? {
        guard let vc = terminalVC else { return nil }
        switch method {
        case "system.ping":
            return ["status": "ok"]

        case "terminal.exec":
            guard let command = params["command"] as? String else {
                return nil
            }
            vc.execCommand(command)
            return ["ok": true]

        case "terminal.feed":
            guard let text = params["text"] as? String else {
                return nil
            }
            vc.feedText(text)
            return ["ok": true]

        case "terminal.state":
            return vc.terminalState()

        case "terminal.read":
            return vc.readScreen()

        default:
            return nil
        }
    }

    // MARK: - Menu Bar

    private func setupMenuBar() {
        let mainMenu = NSMenu()

        // App menu
        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenuItem.submenu = appMenu
        appMenu.addItem(withTitle: "Quit turm", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")

        // View menu (zoom)
        let viewMenuItem = NSMenuItem()
        mainMenu.addItem(viewMenuItem)
        let viewMenu = NSMenu(title: "View")
        viewMenuItem.submenu = viewMenu

        let zoomInItem = NSMenuItem(title: "Zoom In", action: #selector(zoomIn), keyEquivalent: "=")
        zoomInItem.target = self
        viewMenu.addItem(zoomInItem)

        let zoomOutItem = NSMenuItem(title: "Zoom Out", action: #selector(zoomOut), keyEquivalent: "-")
        zoomOutItem.target = self
        viewMenu.addItem(zoomOutItem)

        let zoomResetItem = NSMenuItem(title: "Actual Size", action: #selector(zoomReset), keyEquivalent: "0")
        zoomResetItem.target = self
        viewMenu.addItem(zoomResetItem)

        NSApp.mainMenu = mainMenu
    }

    // MARK: - Zoom Actions

    @objc func zoomIn() {
        terminalVC?.zoomIn()
    }

    @objc func zoomOut() {
        terminalVC?.zoomOut()
    }

    @objc func zoomReset() {
        terminalVC?.zoomReset()
    }
}
