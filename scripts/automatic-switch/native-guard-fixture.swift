// Opt-in disposable AppKit app. No API in this program can target Claude.
import AppKit

final class FixtureDelegate: NSObject, NSApplicationDelegate {
    let directory: URL
    let mode: String
    var lifecycle = FixtureLifecycle()
    var window: NSWindow?
    var alert: NSAlert?
    var timer: Timer?
    var events: [String] = []
    let deadline = Date().addingTimeInterval(20)

    init(mode: String, directory: URL) { self.mode = mode; self.directory = directory }

    func mark(_ name: String) {
        events.append(name)
        try? events.joined(separator: "\n").write(to: directory.appendingPathComponent("events"),
                                                  atomically: true, encoding: .utf8)
        _ = FileManager.default.createFile(atPath: directory.appendingPathComponent(name).path,
                                           contents: Data())
    }
    func exists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent(name).path)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard lifecycle.takePresentation() else { return }
        let window = NSWindow(contentRect: NSRect(x: 80, y: 80, width: 520, height: 170),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "CSW TEST FIXTURE: one disposable dialog"
        let label = NSTextField(labelWithString: "TEST ONLY\nThis app does not control Claude.\nOne dialog, no automatic retries.")
        label.frame = NSRect(x: 24, y: 30, width: 470, height: 110)
        label.font = .boldSystemFont(ofSize: 20)
        window.contentView?.addSubview(label)
        self.window = window
        let alert = NSAlert()
        alert.alertStyle = .warning
        let japanese = mode == "sheet-ja"
        alert.messageText = mode == "unknown" ? "Unrecognized fixture dialog" :
            (japanese ? "Claude はまだ作業中です" : "Claude is still working")
        alert.informativeText = japanese ?
            "Claude は 1 件のセッションで作業中です。今終了すると、その作業が中断されます。" :
            "Claude is working in 3 sessions. Quitting now will interrupt that work."
        for title in japanese ? ["このまま終了", "Claude を待つ", "キャンセル"] :
            ["Quit anyway", "Wait for Claude", "Cancel"] {
            alert.addButton(withTitle: title)
        }
        alert.buttons[1].keyEquivalent = "\r"
        alert.buttons[2].keyEquivalent = "\u{1b}"
        alert.window.title = "CSW TEST FIXTURE"
        alert.icon = NSImage(size: NSSize(width: 96, height: 96), flipped: false) { rect in
            NSColor.systemBlue.setFill()
            NSBezierPath(roundedRect: rect, xRadius: 14, yRadius: 14).fill()
            NSString(string: "TEST").draw(at: NSPoint(x: 8, y: 32), withAttributes: [
                .font: NSFont.boldSystemFont(ofSize: 28), .foregroundColor: NSColor.white
            ])
            return true
        }
        self.alert = alert
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.pollControls()
        }
        window.orderFront(nil)
        mark("presented")
        // Each invocation creates exactly one sheet. A failed reader never
        // relaunches it, and shutdown closes it before exiting this process.
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, self.lifecycle.acceptsSelection else { return }
            if response == .alertSecondButtonReturn {
                guard self.lifecycle.takeWait() else { return }
                self.mark("pressed-1")
            } else {
                self.mark(response == .alertFirstButtonReturn ? "pressed-0" : "pressed-2")
                self.finish()
            }
        }
        mark("ready")
    }

    func pollControls() {
        if exists("shutdown") || Date() >= deadline || getppid() == 1 {
            finish()
            return
        }
        if lifecycle.waiting && ["turn-done", "input-done", "background-done"].allSatisfy(exists) {
            mark("safe-exit")
            finish()
        }
    }

    func finish() {
        guard lifecycle.takeFinish() else { return }
        timer?.invalidate()
        timer = nil
        // Do not repeatedly call NSApplication.terminate while a sheet is open.
        // This app simply closes its own windows and exits its own main loop.
        if let sheet = window?.attachedSheet { window?.endSheet(sheet, returnCode: .abort) }
        alert?.window.orderOut(nil)
        window?.orderOut(nil)
        mark("finished")
        exit(0)
    }
}

@main
struct NativeGuardFixture {
    static func main() {
        let args = CommandLine.arguments
        guard args.count == 4, args[1] == "--one-disposable-dialog",
              ["sheet-en", "sheet-ja", "unknown"].contains(args[2]) else { exit(2) }
        let directory = URL(fileURLWithPath: args[3], isDirectory: true)
        guard FileManager.default.fileExists(atPath: directory.path) else { exit(2) }
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let delegate = FixtureDelegate(mode: args[2], directory: directory)
        app.delegate = delegate
        app.run()
        withExtendedLifetime(delegate) {}
    }
}
