import CoreGraphics
import Darwin
import Metal

let session = CGSessionCopyCurrentDictionary() as? [String: Any]
let onConsole = session?[kCGSessionOnConsoleKey as String] as? Bool ?? false
if MTLCreateSystemDefaultDevice() == nil { exit(78) }
exit(onConsole ? 0 : 77)
