// wincap <windowID> <out.png> : capture ONE window (even if occluded / in the Stage Manager strip) via ScreenCaptureKit.
import Foundation
import AppKit
import ScreenCaptureKit
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

_ = NSApplication.shared
let wid = CGWindowID(UInt32(CommandLine.arguments[1])!)
let out = URL(fileURLWithPath: CommandLine.arguments[2])
let sem = DispatchSemaphore(value: 0)
var code: Int32 = 1
Task {
  do {
    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
    guard let win = content.windows.first(where: { $0.windowID == wid }) else { print("no such window"); sem.signal(); return }
    let filter = SCContentFilter(desktopIndependentWindow: win)
    let cfg = SCStreamConfiguration()
    cfg.width = Int(win.frame.width * 2); cfg.height = Int(win.frame.height * 2)
    cfg.showsCursor = false
    cfg.ignoreShadowsSingleWindow = true
    cfg.capturesShadowsOnly = false
    let img = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg)
    let dest = CGImageDestinationCreateWithURL(out as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, img, nil)
    CGImageDestinationFinalize(dest)
    print("ok \(img.width)x\(img.height) frame=\(win.frame) onScreen=\(win.isOnScreen)")
    code = 0
  } catch { print("error: \(error)") }
  sem.signal()
}
sem.wait()
exit(code)
