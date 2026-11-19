// Writes tiny phone/camera-style inputs for PLAN.md T15.2 (run once on the Mac; outputs are
// committed): portrait.mov (landscape pixels with a 90° preferredTransform), vfr.mov (variable
// frame durations, each frame a distinct grey) and p3.mov (Display P3 primaries).
//   swift fixtures/media/camera-footage.swift fixtures/media
import AVFoundation
import CoreVideo
import Foundation

func write(_ url: URL, size: (Int, Int), times: [CMTime], transform: CGAffineTransform = .identity,
           primaries: String = AVVideoColorPrimaries_ITU_R_709_2,
           paint: (Int, Int, Int) -> (UInt8, UInt8, UInt8)) throws {
    try? FileManager.default.removeItem(at: url)
    let (width, height) = size
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: width,
        AVVideoHeightKey: height,
        AVVideoColorPropertiesKey: [
            AVVideoColorPrimariesKey: primaries,
            AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
        ],
    ])
    input.transform = transform
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
    ])
    writer.add(input)
    precondition(writer.startWriting(), String(describing: writer.error))
    writer.startSession(atSourceTime: .zero)
    for (index, time) in times.enumerated() {
        var buffer: CVPixelBuffer?
        precondition(CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer) == kCVReturnSuccess)
        let picture = buffer!
        // The pixel values are in the tagged primaries.
        CVBufferSetAttachment(picture, kCVImageBufferColorPrimariesKey,
                              primaries == AVVideoColorPrimaries_P3_D65
                                  ? kCVImageBufferColorPrimaries_P3_D65 : kCVImageBufferColorPrimaries_ITU_R_709_2,
                              .shouldPropagate)
        CVPixelBufferLockBaseAddress(picture, [])
        let bytes = CVPixelBufferGetBaseAddress(picture)!.assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(picture)
        for y in 0..<height {
            for x in 0..<width {
                let (r, g, b) = paint(index, x, y)
                let at = y * stride + x * 4
                bytes[at] = b
                bytes[at + 1] = g
                bytes[at + 2] = r
                bytes[at + 3] = 255
            }
        }
        CVPixelBufferUnlockBaseAddress(picture, [])
        while !input.isReadyForMoreMediaData { usleep(1000) }
        precondition(adaptor.append(picture, withPresentationTime: time))
    }
    input.markAsFinished()
    let done = DispatchSemaphore(value: 0)
    writer.endSession(atSourceTime: times.last! + CMTime(value: 1001, timescale: 30000))
    writer.finishWriting { done.signal() }
    done.wait()
    precondition(writer.status == .completed, String(describing: writer.error))
}

let folder = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
let thirty = (0..<30).map { CMTime(value: Int64($0) * 1001, timescale: 30000) }
// Portrait: stored 64×36 landscape, left half red and right half blue; rotated 90° clockwise for
// display, so the top of the 36×64 picture is red and the bottom blue.
try write(folder.appendingPathComponent("portrait.mov"), size: (64, 36), times: thirty,
          transform: CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: 36, ty: 0)) { _, x, _ in
    x < 32 ? (220, 30, 30) : (30, 30, 220)
}
// VFR: durations cycle 1001, 2002, 500 (1/30000 s); frame n is grey 20 + 8n.
var at: Int64 = 0
var vfr: [CMTime] = []
for n in 0..<24 {
    vfr.append(CMTime(value: at, timescale: 30000))
    at += [1001, 2002, 500][n % 3]
}
try write(folder.appendingPathComponent("vfr.mov"), size: (32, 18), times: vfr) { n, _, _ in
    let grey = UInt8(20 + 8 * n)
    return (grey, grey, grey)
}
// P3: a warm orange inside both gamuts; in Rec. 709 the same color has more red and less green.
try write(folder.appendingPathComponent("p3.mov"), size: (32, 18), times: thirty,
          primaries: AVVideoColorPrimaries_P3_D65) { _, _, _ in (200, 120, 60) }
print("wrote portrait.mov, vfr.mov, p3.mov")
