// AC-EXP-2 helpers for tools/av-drift.sh, compiled with swiftc on the Mac.
//   av-drift make <out.mov> [seconds=600]   60 fps 1280x720 H.264 + 48 kHz stereo LPCM: black
//                                           frames with a white one at every whole second, and
//                                           a 2 ms click starting exactly at every whole second
//   av-drift measure <movie> [from=540]     per second ≥ from: audio click onset − flash frame
//                                           time; prints JSON (max |offset| in ms and frames)
import AVFoundation

let args = CommandLine.arguments
let fps = 60, rate = 48_000, width = 1280, height = 720

func run(_ body: @escaping () async throws -> Void) {
    let done = DispatchSemaphore(value: 0)
    Task {
        do { try await body() } catch { FileHandle.standardError.write("\(error)\n".data(using: .utf8)!); exit(1) }
        done.signal()
    }
    done.wait()
}

func make(_ path: String, _ seconds: Int) throws {
    let url = URL(fileURLWithPath: path)
    try? FileManager.default.removeItem(at: url)
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let video = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height])
    let pixels = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: video, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height])
    let audio = AVAssetWriterInput(mediaType: .audio, outputSettings: [
        AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: rate, AVNumberOfChannelsKey: 2,
        AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false])
    video.expectsMediaDataInRealTime = false
    audio.expectsMediaDataInRealTime = false
    writer.add(video)
    writer.add(audio)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    var format: CMAudioFormatDescription?
    var asbd = AudioStreamBasicDescription(mSampleRate: Float64(rate), mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
        mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 2,
        mBitsPerChannel: 16, mReserved: 0)
    CMAudioFormatDescriptionCreate(allocator: nil, asbd: &asbd, layoutSize: 0, layout: nil,
        magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
    // Each input pulls its own samples when the writer asks (pushing both from one loop stalls
    // when the encoder's lookahead makes the writer wait for the other track).
    let chunk = rate / fps, click = rate / 500, total = seconds * fps
    let group = DispatchGroup()
    var nextAudio = 0, nextVideo = 0
    group.enter()
    audio.requestMediaDataWhenReady(on: DispatchQueue(label: "audio")) {
        while audio.isReadyForMoreMediaData && nextAudio < total {
            var samples = [Int16](repeating: 0, count: chunk * 2)
            if nextAudio % fps == 0 { for i in 0..<click { samples[2 * i] = 16_000; samples[2 * i + 1] = 16_000 } }
            var block: CMBlockBuffer?
            CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: samples.count * 2,
                blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: samples.count * 2,
                flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block)
            samples.withUnsafeBytes { _ = CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block!, offsetIntoDestination: 0, dataLength: $0.count) }
            var sample: CMSampleBuffer?
            CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: nil, dataBuffer: block!, formatDescription: format!,
                sampleCount: chunk, presentationTimeStamp: CMTime(value: CMTimeValue(nextAudio * chunk), timescale: CMTimeScale(rate)),
                packetDescriptions: nil, sampleBufferOut: &sample)
            audio.append(sample!)
            nextAudio += 1
        }
        if nextAudio == total { audio.markAsFinished(); nextAudio += 1; group.leave() }
    }
    group.enter()
    video.requestMediaDataWhenReady(on: DispatchQueue(label: "video")) {
        while video.isReadyForMoreMediaData && nextVideo < total {
            var buffer: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, pixels.pixelBufferPool!, &buffer)
            let pb = buffer!
            CVPixelBufferLockBaseAddress(pb, [])
            memset(CVPixelBufferGetBaseAddress(pb)!, nextVideo % fps == 0 ? 255 : 0, CVPixelBufferGetDataSize(pb))
            CVPixelBufferUnlockBaseAddress(pb, [])
            pixels.append(pb, withPresentationTime: CMTime(value: CMTimeValue(nextVideo), timescale: CMTimeScale(fps)))
            nextVideo += 1
        }
        if nextVideo == total { video.markAsFinished(); nextVideo += 1; group.leave() }
    }
    group.wait()
    if writer.status == .failed { throw writer.error! }
    let finished = DispatchSemaphore(value: 0)
    writer.finishWriting { finished.signal() }
    finished.wait()
    if writer.status != .completed { throw writer.error! }
}


func measure(_ path: String, _ from: Double) async throws {
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    let duration = try await asset.load(.duration).seconds
    let range = CMTimeRange(start: CMTime(seconds: max(0, from - 0.5), preferredTimescale: 600), end: .positiveInfinity)
    // Flash frames: mean of a sparse luma grid over the middle half above mid-grey.
    let videoTrack = try await asset.loadTracks(withMediaType: .video)[0]
    let frameRate = Double(try await videoTrack.load(.nominalFrameRate))
    let vr = try AVAssetReader(asset: asset)
    vr.timeRange = range
    let vo = AVAssetReaderTrackOutput(track: videoTrack, outputSettings: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange])
    vr.add(vo)
    vr.startReading()
    var flashes: [Double] = []
    while let sb = vo.copyNextSampleBuffer() {
        guard let pb = CMSampleBufferGetImageBuffer(sb) else { continue }
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        let base = CVPixelBufferGetBaseAddressOfPlane(pb, 0)!.assumingMemoryBound(to: UInt8.self)
        let (w, h, stride) = (CVPixelBufferGetWidthOfPlane(pb, 0), CVPixelBufferGetHeightOfPlane(pb, 0), CVPixelBufferGetBytesPerRowOfPlane(pb, 0))
        var sum = 0, n = 0
        for y in Swift.stride(from: h / 4, to: 3 * h / 4, by: h / 16) { for x in Swift.stride(from: w / 4, to: 3 * w / 4, by: w / 16) { sum += Int(base[y * stride + x]); n += 1 } }
        CVPixelBufferUnlockBaseAddress(pb, .readOnly)
        if sum / n > 128 { flashes.append(CMSampleBufferGetPresentationTimeStamp(sb).seconds) }
    }
    // Click onsets: first sample above half the click level after silence.
    let ar = try AVAssetReader(asset: asset)
    ar.timeRange = range
    let ao = AVAssetReaderTrackOutput(track: try await asset.loadTracks(withMediaType: .audio)[0], outputSettings: [
        AVFormatIDKey: kAudioFormatLinearPCM, AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true,
        AVLinearPCMIsNonInterleaved: false, AVNumberOfChannelsKey: 1])
    ar.add(ao)
    ar.startReading()
    var onsets: [Double] = []
    var lastOnset = -1.0
    var outRate = Double(rate)
    while let sb = ao.copyNextSampleBuffer() {
        if let desc = CMSampleBufferGetFormatDescription(sb), let a = CMAudioFormatDescriptionGetStreamBasicDescription(desc) { outRate = a.pointee.mSampleRate }
        let start = CMSampleBufferGetPresentationTimeStamp(sb).seconds
        var length = 0
        var data: UnsafeMutablePointer<Int8>?
        let block = CMSampleBufferGetDataBuffer(sb)!
        var contiguous: CMBlockBuffer?
        CMBlockBufferCreateContiguous(allocator: nil, sourceBuffer: block, blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: 0, flags: 0, blockBufferOut: &contiguous)
        CMBlockBufferGetDataPointer(contiguous!, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &length, dataPointerOut: &data)
        let floats = UnsafeRawPointer(data!).assumingMemoryBound(to: Float.self)
        for i in 0..<(length / 4) where abs(floats[i]) > 0.24 {
            let t = start + Double(i) / outRate
            if t - lastOnset > 0.5 { onsets.append(t) }
            lastOnset = t
        }
    }
    var worst = 0.0, offsets: [Double] = []
    for flash in flashes where flash >= from - 0.01 {
        guard let onset = onsets.min(by: { abs($0 - flash) < abs($1 - flash) }) else { continue }
        let offset = onset - flash
        offsets.append(offset)
        worst = max(worst, abs(offset))
    }
    let mean = offsets.isEmpty ? 0 : offsets.reduce(0, +) / Double(offsets.count)
    print(String(format: "{\"movie\":\"%@\",\"duration\":%.3f,\"fps\":%.2f,\"from\":%.0f,\"pairs\":%d,\"flashes\":%d,\"clicks\":%d,\"meanOffsetMs\":%.3f,\"maxAbsOffsetMs\":%.3f,\"maxAbsOffsetFrames\":%.3f}",
        (path as NSString).lastPathComponent, duration, frameRate, from, offsets.count, flashes.filter { $0 >= from - 0.01 }.count,
        onsets.filter { $0 >= from - 0.01 }.count, mean * 1000, worst * 1000, worst * frameRate))
}

switch (args.count > 2 ? args[1] : "") {
case "make":
    do { try make(args[2], args.count > 3 ? Int(args[3])! : 600) } catch { FileHandle.standardError.write("\(error)\n".data(using: .utf8)!); exit(1) }
case "measure": run { try await measure(args[2], args.count > 3 ? Double(args[3])! : 540) }
default:
    FileHandle.standardError.write("usage: av-drift make <out.mov> [seconds] | measure <movie> [from]\n".data(using: .utf8)!)
    exit(2)
}
