import AppKit

@MainActor
final class DemoDelegate: NSObject, NSApplicationDelegate {
    var windows: [NSWindow] = []
    var count = 0
    let countLabel = NSTextField(labelWithString: "Count: 0")

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 300),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable],
                              backing: .buffered, defer: false)
        window.title = "macrun Counter"
        window.isReleasedWhenClosed = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.spacing = 20
        stack.translatesAutoresizingMaskIntoConstraints = false
        countLabel.setAccessibilityIdentifier("counter")
        let increment = NSButton(title: "Increment", target: self, action: #selector(incrementCount))
        let field = NSTextField(string: "")
        field.placeholderString = "Type a message"
        field.setAccessibilityLabel("Message")
        field.widthAnchor.constraint(equalToConstant: 300).isActive = true
        let second = NSButton(title: "Open second window", target: self, action: #selector(openSecond))
        [countLabel, increment, field, second].forEach(stack.addArrangedSubview)
        window.contentView!.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: window.contentView!.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: window.contentView!.centerYAnchor)
        ])
        window.center()
        window.makeKeyAndOrderFront(nil)
        windows.append(window)
    }

    @objc func incrementCount() {
        count += 1
        countLabel.stringValue = "Count: \(count)"
    }

    @objc func openSecond() {
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 180),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "macrun Second Window"
        window.isReleasedWhenClosed = false
        let label = NSTextField(labelWithString: "This is the second window")
        label.frame = NSRect(x: 30, y: 70, width: 270, height: 30)
        window.contentView!.addSubview(label)
        window.makeKeyAndOrderFront(nil)
        windows.append(window)
    }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = DemoDelegate()
    app.setActivationPolicy(.regular)
    app.delegate = delegate
    app.run()
}
