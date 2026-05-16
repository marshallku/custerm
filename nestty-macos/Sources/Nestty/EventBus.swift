import Foundation

/// Mirror of `nestty-core::event_bus::Event`. `data` is `serde_json::Value`-
/// shaped — object, array, scalar, or null all valid.
struct BusEvent {
    let type: String
    let source: String
    let data: Any?
    let timestampMs: UInt64
    /// nil for native broadcasts; non-nil for events republished from the
    /// daemon side. Outbound forwarders (PR 4b) skip events with bridgeId
    /// set to break echo loops.
    let bridgeId: String?
}

/// Local broadcast hub. Subscribers fall into two flavors:
/// - JSON-string channels for legacy consumers (`SocketServer` wire output,
///   `PluginPanelController` JS bridge) — wire shape `{"event", "data"}`.
/// - Typed `BusEvent` channels for consumers that need metadata (PR 4b
///   outbound forwarder gates on `bridgeId`).
///
/// Per-subscriber FIFO; cross-channel order undefined because `onBroadcast`
/// fires synchronously and subscribers may chain reentrant broadcasts.
final class EventBus: @unchecked Sendable {
    private let lock = NSLock()
    private var jsonChannels: [EventChannel] = []
    private var typedChannels: [TypedEventChannel] = []

    /// Synchronous fan-out hook. Set by AppDelegate to forward every event
    /// into the trigger engine without making EventBus aware of `NesttyEngine`.
    /// `source` carries the trust stamp the engine's
    /// `try_promote_or_drop_preflight` gates on (`COMPLETION_EVENT_SOURCE` =
    /// `"nestty.action"`).
    nonisolated(unsafe) var onBroadcast: (@Sendable (
        _ kind: String,
        _ source: String,
        _ data: Any?,
    ) -> Void)?

    func subscribe() -> EventChannel {
        let ch = EventChannel()
        lock.withLock { jsonChannels.append(ch) }
        return ch
    }

    func subscribeTyped() -> TypedEventChannel {
        let ch = TypedEventChannel()
        lock.withLock { typedChannels.append(ch) }
        return ch
    }

    /// Broadcast an event to all live subscribers.
    ///
    /// `bridgeId` is nil for native broadcasts; only set by inbound
    /// daemon-event republish so PR 4b's forwarder can skip echoes.
    /// `timestampMs` defaults to `now`. Daemon wire `Event` does not carry
    /// a timestamp, so republished events also use the local observation
    /// time.
    func broadcast(
        event: String,
        source: String = "macos.eventbus",
        data: Any? = nil,
        bridgeId: String? = nil,
        timestampMs: UInt64? = nil,
    ) {
        let ts = timestampMs ?? UInt64(Date().timeIntervalSince1970 * 1000)
        let busEvent = BusEvent(type: event, source: source, data: data, timestampMs: ts, bridgeId: bridgeId)

        // Engine hop first so a chained broadcast from a trigger callback
        // lands in the same logical tick.
        onBroadcast?(event, source, data)

        // Snapshot under lock so subscribers can reentrantly broadcast
        // without deadlock. Identity-based pruning so a concurrent
        // broadcaster's index-based removal can't drop the wrong channel.
        let (jsonSnap, typedSnap) = lock.withLock { (jsonChannels, typedChannels) }

        // Serialization failure ≠ subscriber death — skip JSON fanout for
        // this one event but keep the subscribers around for future
        // broadcasts. Pruning is driven only by `send` returning false
        // (channel closed).
        var deadJSON: [ObjectIdentifier] = []
        if let line = serializeWire(busEvent) {
            for ch in jsonSnap where !ch.send(line) {
                deadJSON.append(ObjectIdentifier(ch))
            }
        }

        var deadTyped: [ObjectIdentifier] = []
        for ch in typedSnap where !ch.send(busEvent) {
            deadTyped.append(ObjectIdentifier(ch))
        }

        if !deadJSON.isEmpty || !deadTyped.isEmpty {
            lock.withLock {
                if !deadJSON.isEmpty {
                    let dead = Set(deadJSON)
                    jsonChannels.removeAll { dead.contains(ObjectIdentifier($0)) }
                }
                if !deadTyped.isEmpty {
                    let dead = Set(deadTyped)
                    typedChannels.removeAll { dead.contains(ObjectIdentifier($0)) }
                }
            }
        }
    }

    /// `{"event", "data"}` wire (local format). `.fragmentsAllowed` so
    /// scalar / null payloads serialize. Daemon wire uses `type`/`data`;
    /// translation happens at the DaemonClient boundary.
    private func serializeWire(_ event: BusEvent) -> String? {
        let payload: [String: Any] = ["event": event.type, "data": event.data ?? NSNull()]
        guard
            let data = try? JSONSerialization.data(withJSONObject: payload, options: [.fragmentsAllowed]),
            let s = String(data: data, encoding: .utf8)
        else { return nil }
        return s
    }
}

/// Single-subscriber FIFO queue for serialized JSON.
final class EventChannel: @unchecked Sendable {
    private var queue: [String] = []
    private let sema = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var closed = false

    func send(_ event: String) -> Bool {
        lock.lock()
        guard !closed else { lock.unlock(); return false }
        queue.append(event)
        lock.unlock()
        sema.signal()
        return true
    }

    func receive() -> String? {
        sema.wait()
        return lock.withLock {
            if closed, queue.isEmpty { return nil }
            return queue.isEmpty ? nil : queue.removeFirst()
        }
    }

    func close() {
        lock.withLock { closed = true }
        sema.signal()
    }
}

/// Single-subscriber FIFO queue for typed `BusEvent`. Same shape as
/// `EventChannel` so subscribers compose identically.
final class TypedEventChannel: @unchecked Sendable {
    private var queue: [BusEvent] = []
    private let sema = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var closed = false

    func send(_ event: BusEvent) -> Bool {
        lock.lock()
        guard !closed else { lock.unlock(); return false }
        queue.append(event)
        lock.unlock()
        sema.signal()
        return true
    }

    func receive() -> BusEvent? {
        sema.wait()
        return lock.withLock {
            if closed, queue.isEmpty { return nil }
            return queue.isEmpty ? nil : queue.removeFirst()
        }
    }

    func close() {
        lock.withLock { closed = true }
        sema.signal()
    }
}

private extension NSLock {
    @discardableResult
    func withLock<T>(_ body: () -> T) -> T {
        lock(); defer { unlock() }
        return body()
    }
}
