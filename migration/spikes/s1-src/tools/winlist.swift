import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
for w in list {
  let p = w[kCGWindowOwnerPID as String] as? Int32 ?? 0
  if p == pid {
    let b = w[kCGWindowBounds as String] as! [String: Any]
    print("wid=\(w[kCGWindowNumber as String]!) layer=\(w[kCGWindowLayer as String]!) onscreen=\(w[kCGWindowIsOnscreen as String] ?? "-") alpha=\(w[kCGWindowAlpha as String]!) x=\(b["X"]!) y=\(b["Y"]!) w=\(b["Width"]!) h=\(b["Height"]!) owner=\(w[kCGWindowOwnerName as String] ?? "-") name=\(w[kCGWindowName as String] ?? "-")")
  }
}
