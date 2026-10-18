// Shared by the disposable fixture and its window-free regression checks.
struct FixtureLifecycle {
    private enum Phase { case created, presented, waiting, finished }
    private var phase = Phase.created

    var waiting: Bool { phase == .waiting }
    var acceptsSelection: Bool { phase == .presented }

    mutating func takePresentation() -> Bool {
        guard phase == .created else { return false }
        phase = .presented
        return true
    }

    mutating func takeWait() -> Bool {
        guard phase == .presented else { return false }
        phase = .waiting
        return true
    }

    mutating func takeFinish() -> Bool {
        guard phase != .finished else { return false }
        phase = .finished
        return true
    }
}
