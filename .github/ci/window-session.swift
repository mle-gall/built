import AppKit
import CoreGraphics
import Darwin
import Metal

let session = CGSessionCopyCurrentDictionary() as? [String: Any]
let onConsole = session?[kCGSessionOnConsoleKey as String] as? Bool ?? false
if MTLCreateSystemDefaultDevice() == nil { exit(78) }
guard onConsole, let screen = NSScreen.main else { exit(77) }
// The script coordinates are in logical points. Prefer a mode which can
// accommodate the largest scripted window on the hosted display.
let display = CGMainDisplayID()
if screen.visibleFrame.width < 1600 || screen.visibleFrame.height < 1000 {
    if let modes = CGDisplayCopyAllDisplayModes(display, nil) as? [CGDisplayMode] {
        let suitable = modes.filter { $0.width >= 1600 && $0.height >= 1100 }
        if let mode = suitable.min(by: { $0.width * $0.height < $1.width * $1.height }) {
            _ = CGDisplaySetDisplayMode(display, mode, nil)
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.5))
        }
    }
}
guard let bounds = NSScreen.main?.visibleFrame else { exit(77) }
print("\(Int(bounds.width))x\(Int(bounds.height))")
