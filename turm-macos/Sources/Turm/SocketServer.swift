import Darwin
import Foundation

/// Unix socket server that accepts turmctl connections.
/// Mirrors the role of socket.rs in turm-linux.
///
/// Threading model:
///   - Accept loop runs on a dedicated Thread (POSIX blocking)
///   - Each client gets its own Thread
///   - Command handler is always called via DispatchQueue.main.sync
///   - Caller thread blocks on semaphore until main thread returns result
final class SocketServer: @unchecked Sendable {
    private let socketPath: String
    private var serverFd: Int32 = -1

    /// Called on the main thread to dispatch a command.
    /// Returns the result dict, or nil for unknown method.
    var commandHandler: ((_ method: String, _ params: [String: Any]) -> Any?)?

    init() {
        let pid = ProcessInfo.processInfo.processIdentifier
        socketPath = "/tmp/turm-\(pid).sock"
    }

    var path: String {
        socketPath
    }

    func start() {
        unlink(socketPath)

        serverFd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard serverFd >= 0 else {
            print("[turm] socket create failed: \(String(cString: strerror(errno)))")
            return
        }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { buf in
            let count = min(socketPath.utf8.count, buf.count - 1)
            buf.copyBytes(from: socketPath.utf8.prefix(count))
            buf[count] = 0
        }

        let addrSize = socklen_t(MemoryLayout<sockaddr_un>.size)
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(serverFd, $0, addrSize)
            }
        }

        guard bound == 0 else {
            print("[turm] socket bind failed: \(String(cString: strerror(errno)))")
            close(serverFd)
            serverFd = -1
            return
        }

        listen(serverFd, 5)
        print("[turm] socket listening at \(socketPath)")

        let fd = serverFd
        Thread.detachNewThread { [weak self] in self?.acceptLoop(serverFd: fd) }
    }

    func stop() {
        if serverFd >= 0 {
            close(serverFd)
            serverFd = -1
        }
        unlink(socketPath)
    }

    // MARK: - Private

    private func acceptLoop(serverFd: Int32) {
        while true {
            let clientFd = accept(serverFd, nil, nil)
            if clientFd < 0 { break }
            Thread.detachNewThread { [weak self] in self?.handleClient(clientFd) }
        }
    }

    private func handleClient(_ fd: Int32) {
        defer { close(fd) }

        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)

        while true {
            let n = read(fd, &chunk, chunk.count)
            if n <= 0 { break }
            buffer.append(contentsOf: chunk[..<n])

            while let nlIdx = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let line = Data(buffer[..<nlIdx])
                buffer = Data(buffer[buffer.index(after: nlIdx)...])

                var response = dispatch(line)
                response.append(UInt8(ascii: "\n"))
                _ = response.withUnsafeBytes { write(fd, $0.baseAddress!, $0.count) }
            }
        }
    }

    private func dispatch(_ data: Data) -> Data {
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let id = json["id"] as? String,
            let method = json["method"] as? String
        else {
            return error(id: "?", code: "invalid_request", message: "malformed JSON")
        }

        let params = json["params"] as? [String: Any] ?? [:]

        // Call handler on main thread and wait synchronously
        let outcome: (result: Any, ok: Bool) = DispatchQueue.main.sync {
            guard let result = self.commandHandler?(method, params) else {
                return (["code": "unknown_method", "message": "unknown: \(method)"] as [String: Any], false)
            }
            return (result, true)
        }

        if outcome.ok {
            return success(id: id, result: outcome.result)
        } else {
            let err = outcome.result as? [String: Any] ?? [:]
            return error(
                id: id,
                code: err["code"] as? String ?? "error",
                message: err["message"] as? String ?? "unknown error",
            )
        }
    }

    private func success(id: String, result: Any) -> Data {
        let dict: [String: Any] = ["id": id, "ok": true, "result": result]
        return (try? JSONSerialization.data(withJSONObject: dict)) ?? Data()
    }

    private func error(id: String, code: String, message: String) -> Data {
        let dict: [String: Any] = [
            "id": id,
            "ok": false,
            "error": ["code": code, "message": message],
        ]
        return (try? JSONSerialization.data(withJSONObject: dict)) ?? Data()
    }
}
