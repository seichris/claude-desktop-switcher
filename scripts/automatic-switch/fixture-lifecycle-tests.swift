import Foundation

func check(_ condition: Bool, _ message: String = "lifecycle invariant failed") {
    if !condition {
        FileHandle.standardError.write(Data("FAIL \(message)\n".utf8))
        exit(1)
    }
}

@main
struct LifecycleTests {
    static func main() {
        var fixture = FixtureLifecycle()
        let presentations = (0..<100).filter { _ in fixture.takePresentation() }.count
        check(presentations == 1, "fixture must present once, observed \(presentations)")
        let finishes = (0..<100).filter { _ in fixture.takeFinish() }.count
        check(finishes == 1, "fixture must finish once, observed \(finishes)")
        check(!fixture.takePresentation(), "finished fixture must never reopen")
        var cancelledBeforePresentation = FixtureLifecycle()
        check(cancelledBeforePresentation.takeFinish())
        check(!cancelledBeforePresentation.takePresentation())
        var waiting = FixtureLifecycle()
        check(!waiting.takeWait())
        check(waiting.takePresentation())
        check(waiting.acceptsSelection)
        check(waiting.takeWait())
        check(waiting.waiting)
        check(!waiting.takeWait())
        check(!waiting.takePresentation())
        check(waiting.takeFinish())
        check(!waiting.waiting && !waiting.takeWait() && !waiting.takeFinish())
        check(!waiting.acceptsSelection, "shutdown callback must not fabricate Cancel evidence")
        print("PASS fixture lifecycle: one presentation, one finish, wait once, no reopening")
    }
}
