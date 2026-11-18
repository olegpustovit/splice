// Builds playable synthetic inputs for the descriptor-only sample and counts encoded MP4 frames.
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import ImageIO
import UniformTypeIdentifiers

func movie(_ url: URL, height: Int, blue: UInt8) throws {
    let width = 640
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: width,
        AVVideoHeightKey: height
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(
        assetWriterInput: input,
        sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height
        ])
    writer.add(input)
    precondition(writer.startWriting(), String(describing: writer.error))
    writer.startSession(atSourceTime: .zero)
    var buffer: CVPixelBuffer?
    let allocated = CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer)
    precondition(allocated == kCVReturnSuccess)
    let picture = buffer!
    CVPixelBufferLockBaseAddress(picture, [])
    let bytes = CVPixelBufferGetBaseAddress(picture)!.assumingMemoryBound(to: UInt8.self)
    let stride = CVPixelBufferGetBytesPerRow(picture)
    for y in 0..<height {
        for x in 0..<width {
            let offset = y * stride + x * 4
            bytes[offset] = blue
            bytes[offset + 1] = UInt8(x * 150 / width + 50)
            bytes[offset + 2] = UInt8(y * 160 / height + 40)
            bytes[offset + 3] = 255
        }
    }
    CVPixelBufferUnlockBaseAddress(picture, [])
    let deadline = Date().addingTimeInterval(20)
    for frame in 0..<540 {
        while !input.isReadyForMoreMediaData {
            precondition(Date() < deadline && writer.status == .writing, String(describing: writer.error))
            Thread.sleep(forTimeInterval: 0.001)
        }
        precondition(adaptor.append(picture, withPresentationTime: CMTime(value: Int64(frame), timescale: 30)),
                     String(describing: writer.error))
    }
    writer.endSession(atSourceTime: CMTime(value: 540, timescale: 30))
    input.markAsFinished()
    let finished = DispatchSemaphore(value: 0)
    writer.finishWriting { finished.signal() }
    precondition(finished.wait(timeout: .now() + 20) == .success && writer.status == .completed,
                 String(describing: writer.error))
}

func wave(_ url: URL) throws {
    let audio = try AVAudioFile(forWriting: url, settings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC,
        AVSampleRateKey: 48_000,
        AVNumberOfChannelsKey: 2
    ])
    let buffer = AVAudioPCMBuffer(pcmFormat: audio.processingFormat, frameCapacity: 48_000)!
    buffer.frameLength = 48_000
    for channel in 0..<2 {
        let samples = buffer.floatChannelData![channel]
        for index in 0..<48_000 {
            samples[index] = 0.1 * sin(Float(index) * 2 * .pi * 440 / 48_000)
        }
    }
    for _ in 0..<18 { try audio.write(from: buffer) }
}

func fixture(at output: URL) throws {
    let fs = FileManager.default
    try fs.copyItem(at: URL(fileURLWithPath: "fixtures/projects/sample.splice"), to: output)
    let assets = output.appendingPathComponent("assets", isDirectory: true)
    try fs.createDirectory(at: assets, withIntermediateDirectories: false)
    let opener = assets.appendingPathComponent("opener.mov")
    try movie(opener, height: 360, blue: 65)
    try movie(assets.appendingPathComponent("screen.mov"), height: 400, blue: 145)
    for name in ["camera.mov", "webcam.mov", "closing.mov", "broll.mov"] {
        try fs.linkItem(at: opener, to: assets.appendingPathComponent(name))
    }
    let audio = assets.appendingPathComponent("mic.m4a")
    try wave(audio)
    try fs.linkItem(at: audio, to: assets.appendingPathComponent("system.m4a"))

    let context = CGContext(data: nil, width: 320, height: 180, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.setFillColor(red: 0.9, green: 0.6, blue: 0.2, alpha: 0.8)
    context.fillEllipse(in: CGRect(x: 80, y: 20, width: 160, height: 140))
    let logo = assets.appendingPathComponent("logo.png") as CFURL
    let image = CGImageDestinationCreateWithURL(logo, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(image, context.makeImage()!, nil)
    precondition(CGImageDestinationFinalize(image))
}

func verify(_ url: URL, fourCC: String) throws {
    let asset = AVURLAsset(url: url, options: [AVURLAssetPreferPreciseDurationAndTimingKey: true])
    let track = asset.tracks(withMediaType: .video).first!
    let format = track.formatDescriptions.first! as! CMFormatDescription
    let codec = CMFormatDescriptionGetMediaSubType(format)
    let label = String(bytes: [UInt8(codec >> 24), UInt8((codec >> 16) & 255),
                               UInt8((codec >> 8) & 255), UInt8(codec & 255)], encoding: .ascii)!
    precondition(label == fourCC, "Expected \(fourCC), got \(label)")
    precondition(track.naturalSize == CGSize(width: 1080, height: 1920), "Wrong export dimensions")
    precondition(CMTimeCompare(track.timeRange.duration, CMTime(value: 660, timescale: 30)) == 0,
                 "Wrong video duration \(track.timeRange.duration.seconds)")
    precondition(!asset.tracks(withMediaType: .audio).isEmpty, "Missing audio track")
    let reader = try AVAssetReader(asset: asset)
    let frames = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
    reader.add(frames)
    precondition(reader.startReading(), String(describing: reader.error))
    var count = 0
    while let sample = frames.copyNextSampleBuffer() {
        count += CMSampleBufferGetNumSamples(sample)
    }
    precondition(reader.status == .completed && count == 660, "Expected 660 frames, got \(count)")
    print("\(label) \(count) frames, \(track.timeRange.duration.seconds) s, \(track.naturalSize)")
}

let args = CommandLine.arguments
let path = URL(fileURLWithPath: args[2])
switch args[1] {
case "make":
    try fixture(at: path)
case "verify":
    try verify(path, fourCC: args[3])
default:
    fatalError("Expected make or verify")
}
