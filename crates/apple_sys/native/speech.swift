// On-device transcription (PLAN.md T9.1) with SpeechAnalyzer, exported to Rust as a C function.
// Swift-only Apple APIs reach Rust the same way as the Objective-C shims: `@_cdecl` gives a C
// symbol that platform.h declares. Output is one word per line: "start\tend\ttext" in seconds.
import AVFoundation
import Foundation
import Speech

@_cdecl("splice_speech_transcribe")
public func spliceSpeechTranscribe(
    _ path: UnsafePointer<CChar>,
    _ locale: UnsafePointer<CChar>,
    _ out: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>
) -> Bool {
    guard #available(macOS 26.0, *) else {
        error.pointee = strdup("Transcription needs macOS 26 or later.")
        return false
    }
    let url = URL(fileURLWithPath: String(cString: path))
    let requested = Locale(identifier: String(cString: locale))
    let done = DispatchSemaphore(value: 0)
    let box = Outcome()
    // The caller may be the main thread: run on the cooperative pool and wait.
    Task.detached {
        do {
            box.text = try await transcribe(url, requested)
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
    error.pointee = strdup(box.failure ?? "Transcription failed.")
    return false
}

private final class Outcome: @unchecked Sendable {
    var text: String?
    var failure: String?
}

private struct SpeechError: Error, CustomStringConvertible {
    let description: String
}

@available(macOS 26.0, *)
private func transcribe(_ url: URL, _ requested: Locale) async throws -> String {
    guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requested) else {
        throw SpeechError(description: "No on-device speech model for \(requested.identifier).")
    }
    let transcriber = SpeechTranscriber(
        locale: locale, transcriptionOptions: [], reportingOptions: [],
        attributeOptions: [.audioTimeRange])
    if let install = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
        try await install.downloadAndInstall()
    }
    let file = try AVAudioFile(forReading: url)
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    let collect = Task { () throws -> String in
        var lines = ""
        for try await result in transcriber.results where result.isFinal {
            for run in result.text.runs {
                guard let range = run.audioTimeRange else { continue }
                let word = String(result.text[run.range].characters)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                    .replacingOccurrences(of: "\t", with: " ")
                    .replacingOccurrences(of: "\n", with: " ")
                if word.isEmpty { continue }
                lines += "\(range.start.seconds)\t\(range.end.seconds)\t\(word)\n"
            }
        }
        return lines
    }
    if let last = try await analyzer.analyzeSequence(from: file) {
        try await analyzer.finalizeAndFinish(through: last)
    } else {
        await analyzer.cancelAndFinishNow()
    }
    return try await collect.value
}
