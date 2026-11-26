import AppKit
import CoreGraphics
import Darwin
import Metal

// `window-session.swift configure` asks the hosted display for a mode that
// holds the largest scripted window, for the rest of the login session.
// `window-session.swift` alone prints the visible frame in logical points.
// They are two processes on purpose: a mode set with CGDisplaySetDisplayMode
// lasts only as long as the process that set it, so the size printed by that
// process was not the size the tests then found.
let session = CGSessionCopyCurrentDictionary() as? [String: Any]
let onConsole = session?[kCGSessionOnConsoleKey as String] as? Bool ?? false
if MTLCreateSystemDefaultDevice() == nil { exit(78) }
guard onConsole, NSScreen.main != nil else { exit(77) }

if CommandLine.arguments.dropFirst().first == "configure" {
    let display = CGMainDisplayID()
    let options = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
    let modes = (CGDisplayCopyAllDisplayModes(display, options) as? [CGDisplayMode]) ?? []
    // Logical points: the scripts' coordinates are in points.
    let suitable = modes.filter { $0.width >= 1600 && $0.height >= 1100 }
    if let mode = suitable.min(by: { $0.width * $0.height < $1.width * $1.height }) {
        var config: CGDisplayConfigRef?
        if CGBeginDisplayConfiguration(&config) == .success {
            CGConfigureDisplayWithDisplayMode(config, display, mode, nil)
            let result = CGCompleteDisplayConfiguration(config, .forSession)
            FileHandle.standardError.write("display mode \(mode.width)x\(mode.height): \(result == .success ? "set" : "refused")\n".data(using: .utf8)!)
        }
    } else {
        let sizes = modes.map { "\($0.width)x\($0.height)" }.joined(separator: " ")
        FileHandle.standardError.write("no display mode of 1600x1100 or more; offered: \(sizes)\n".data(using: .utf8)!)
    }
    exit(0)
}

guard let bounds = NSScreen.main?.visibleFrame else { exit(77) }
print("\(Int(bounds.width))x\(Int(bounds.height))")
