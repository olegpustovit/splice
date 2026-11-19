// Muxes a speech file under a looped video fixture into one movie (tools/speed-check.py).
//   swift tools/speech-movie.swift <video.mov> <speech.m4a> <out.mov>
import AVFoundation

let args = CommandLine.arguments
let video = AVURLAsset(url: URL(fileURLWithPath: args[1]))
let speech = AVURLAsset(url: URL(fileURLWithPath: args[2]))
let out = URL(fileURLWithPath: args[3])
try? FileManager.default.removeItem(at: out)
let done = DispatchSemaphore(value: 0)
Task {
    let composition = AVMutableComposition()
    let duration = try await speech.load(.duration)
    let picture = try await video.loadTracks(withMediaType: .video)[0]
    let pictureLength = try await video.load(.duration)
    let videoTrack = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    var at = CMTime.zero
    while at < duration {
        let take = min(pictureLength, duration - at)
        try videoTrack.insertTimeRange(CMTimeRange(start: .zero, duration: take), of: picture, at: at)
        at = at + take
    }
    let voice = try await speech.loadTracks(withMediaType: .audio)[0]
    let audioTrack = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    try audioTrack.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: voice, at: .zero)
    let export = AVAssetExportSession(asset: composition, presetName: AVAssetExportPresetHighestQuality)!
    try await export.export(to: out, as: .mov)
    done.signal()
}
done.wait()
