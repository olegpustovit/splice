// Helpers for tools/longrun-capture.sh (AC-REC-1), compiled with swiftc on the Mac.
//   longrun-capture animate          full-screen window that changes every display frame; on
//                                    SIGTERM prints its own frame count and missed refreshes (JSON)
//   longrun-capture gaps <screen.mov> frame count and timing gaps of a recorded screen track (JSON)
import AppKit
import AVFoundation
import QuartzCore

let args = CommandLine.arguments

func animate() {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let screen = NSScreen.main!
    let window = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
    window.level = .floating
    window.isOpaque = true
    let root = CALayer()
    let bar = CALayer()
    bar.backgroundColor = NSColor.white.cgColor
    bar.frame = CGRect(x: 0, y: 0, width: 80, height: screen.frame.height)
    root.addSublayer(bar)
    window.contentView!.wantsLayer = true
    window.contentView!.layer = root
    window.orderFrontRegardless()
    var tick = 0, missed = 0, last = 0.0
    // One step per display refresh: every captured frame differs from the one before.
    let ticker = Ticker { link in
        if last > 0 { missed += max(0, Int(((link.timestamp - last) / link.duration).rounded()) - 1) }
        last = link.timestamp
        tick += 1
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let hue = CGFloat(tick % 600) / 600
        root.backgroundColor = NSColor(hue: hue, saturation: 0.6, brightness: 0.8, alpha: 1).cgColor
        bar.frame.origin.x = CGFloat((tick * 8) % Int(screen.frame.width))
        CATransaction.commit()
    }
    let link = window.contentView!.displayLink(target: ticker, selector: #selector(Ticker.step(_:)))
    link.add(to: .main, forMode: .common)
    signal(SIGTERM, SIG_IGN)
    let stop = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    stop.setEventHandler {
        print("{\"ticks\":\(tick),\"missedRefreshes\":\(missed)}")
        exit(0)
    }
    stop.resume()
    app.run()
}

final class Ticker: NSObject {
    let body: (CADisplayLink) -> Void
    init(_ body: @escaping (CADisplayLink) -> Void) { self.body = body }
    @objc func step(_ link: CADisplayLink) { body(link) }
}

func gaps(_ path: String) {
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    let done = DispatchSemaphore(value: 0)
    Task {
        let track = try await asset.loadTracks(withMediaType: .video)[0]
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
        reader.add(output)
        reader.startReading()
        var times: [Double] = []
        while let sample = output.copyNextSampleBuffer() {
            if CMSampleBufferGetNumSamples(sample) > 0 {
                times.append(CMSampleBufferGetPresentationTimeStamp(sample).seconds)
            }
        }
        times.sort()
        let period = 1.0 / 60
        var missing = 0, gapCount = 0, longest = 0.0
        for (a, b) in zip(times, times.dropFirst()) {
            let step = b - a
            longest = max(longest, step)
            if step > 1.5 * period {
                gapCount += 1
                missing += Int((step / period).rounded()) - 1
            }
        }
        let span = (times.last ?? 0) - (times.first ?? 0)
        let expected = Int((span / period).rounded()) + 1
        print("{\"frames\":\(times.count),\"spanSeconds\":\(span),\"expected60\":\(expected),"
            + "\"missingSlots\":\(missing),\"gaps\":\(gapCount),\"longestGapMs\":\(longest * 1000)}")
        done.signal()
    }
    done.wait()
}

switch args.dropFirst().first {
case "animate": animate()
case "gaps" where args.count == 3: gaps(args[2])
default:
    FileHandle.standardError.write("usage: longrun-capture animate | gaps <screen.mov>\n".data(using: .utf8)!)
    exit(2)
}
