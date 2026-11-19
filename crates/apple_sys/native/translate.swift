// On-device caption translation (PLAN.md T20.3, D7 fallback) with the Translation framework's
// direct session (macOS 26). Input and output are texts joined by U+001E; the language pack
// must already be installed (downloading one needs UI), otherwise this fails with a fix.
import Foundation
import Translation

@_cdecl("splice_translate")
public func spliceTranslate(
    _ joined: UnsafePointer<CChar>,
    _ source: UnsafePointer<CChar>,
    _ target: UnsafePointer<CChar>,
    _ out: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>
) -> Bool {
    guard #available(macOS 26.0, *) else {
        error.pointee = strdup("On-device translation needs macOS 26 or later.")
        return false
    }
    let texts = String(cString: joined).components(separatedBy: "\u{1e}")
    let from = Locale.Language(identifier: String(cString: source))
    let to = Locale.Language(identifier: String(cString: target))
    let done = DispatchSemaphore(value: 0)
    let box = Translated()
    Task.detached {
        do {
            let session = TranslationSession(installedSource: from, target: to)
            let requests = texts.enumerated().map {
                TranslationSession.Request(sourceText: $0.element, clientIdentifier: String($0.offset))
            }
            var results = [String](repeating: "", count: texts.count)
            for response in try await session.translations(from: requests) {
                if let id = response.clientIdentifier, let index = Int(id) {
                    results[index] = response.targetText
                }
            }
            box.text = results.joined(separator: "\u{1e}")
        } catch {
            box.failure = "\(error)"
        }
        done.signal()
    }
    done.wait()
    if let text = box.text {
        out.pointee = strdup(text)
        return true
    }
    error.pointee = strdup(box.failure ?? "Translation failed.")
    return false
}

private final class Translated: @unchecked Sendable {
    var text: String?
    var failure: String?
}
