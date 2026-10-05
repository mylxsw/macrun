import Cocoa

// Only this disposable window is ever captured by the real-backend probe.
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 1000, height: 700),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "Macrun capture validation fixture"
let label = NSTextField(labelWithString: "Macrun GUL-215 — disposable capture fixture")
label.frame = NSRect(x: 40, y: 600, width: 900, height: 40)
label.font = NSFont.systemFont(ofSize: 25)
window.contentView!.addSubview(label)
for i in 0..<80 {
    let field = NSTextField(labelWithString: "Fixture row \(i) — abcdefghijklmnopqrstuvwxyz 0123456789")
    field.frame = NSRect(x: 30 + (i % 2) * 490, y: 580 - (i / 2) * 13, width: 475, height: 13)
    field.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .regular)
    window.contentView!.addSubview(field)
}
window.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
print("{\"pid\":\(ProcessInfo.processInfo.processIdentifier),\"window_id\":\(window.windowNumber)}")
fflush(stdout)
app.run()
