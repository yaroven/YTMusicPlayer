import Foundation
import MediaPlayer
import YTMKit

// Drives the app's Player on macOS (muted): play, gapless advance, skip,
// radio, history, pause while loading, repeat one. Network; ~1.5 min.
@MainActor func run() async throws {
    let library = Library(file: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("lib-\(UUID()).json"))
    let player = Player(library: library, muted: true)
    let a = Track(videoId: "YkLLcIKhJ64", title: "Dancing Queen", artist: "ABBA", durationSecs: 231)
    let b = Track(videoId: "fJ9rUzIMcZQ", title: "Bohemian Rhapsody", artist: "Queen", durationSecs: 359)
    player.play([a, b])
    func wait(_ what: String, _ seconds: Double = 15, _ ok: () -> Bool) async {
        let end = Date().addingTimeInterval(seconds)
        while !ok() && Date() < end { try? await Task.sleep(for: .milliseconds(200)) }
        print(ok() ? "ok  " : "FAIL", what, "| index", player.index, "pos", String(format: "%.1f", player.position),
              "dur", player.duration, "playing", player.isPlaying, player.error ?? "")
    }
    await wait("first song plays") { player.isPlaying && player.position > 0.5 }
    await wait("real duration") { abs(player.duration - 230.4) < 1 }
    // The lock screen / Control Center: the cover, and ⏸ while it plays.
    func nowPlaying(_ key: String) -> Any? { MPNowPlayingInfoCenter.default().nowPlayingInfo?[key] }
    await wait("now playing has the cover", 10) { nowPlaying(MPMediaItemPropertyArtwork) != nil }
    await wait("now playing says playing") { (nowPlaying(MPNowPlayingInfoPropertyPlaybackRate) as? Double) == 1 }
    await wait("now playing position moves on") {
        abs(((nowPlaying(MPNowPlayingInfoPropertyElapsedPlaybackTime) as? Double) ?? -10) - player.position) < 3
    }
    await wait("history") { library.history.first?.videoId == a.videoId }
    await wait("radio appended (autoplay)") { player.queue.count > 2 }
    player.seek(to: 226)
    await wait("gapless advance to second", 12) { player.index == 1 && player.position > 0.3 }
    await wait("second song's duration") { abs(player.duration - 359) < 3 }
    player.next()
    await wait("skip to radio song") { player.index == 2 && player.isPlaying }
    player.previous()  // position < 3: back to song 1
    await wait("previous") { player.index == 1 && player.isPlaying }
    player.togglePlay()
    await wait("paused") { !player.isPlaying }
    await wait("now playing says paused") { (nowPlaying(MPNowPlayingInfoPropertyPlaybackRate) as? Double) == 0 }
    let at = player.position
    try await Task.sleep(for: .seconds(2))
    await wait("stays paused") { !player.isPlaying && abs(player.position - at) < 0.1 }
    player.jump(to: 3)
    player.togglePlay()  // pause while resolving
    try await Task.sleep(for: .seconds(4))
    await wait("pause while resolving holds") { player.index == 3 && !player.isPlaying && player.position < 0.1 }
    player.togglePlay()
    await wait("resumes") { player.isPlaying && player.position > 0.3 }
    player.jump(to: 1)
    await wait("jump") { player.index == 1 && player.position > 0.3 }
    player.repeatMode = .one
    player.seek(to: 355)
    await wait("repeat one restarts", 12) { player.index == 1 && player.position < 3 && player.position > 0 }
    player.repeatMode = .off

    // Sleep timer, end of song: the next song is reached but doesn't play.
    player.setSleep(minutes: nil)
    player.seek(to: 354)
    await wait("sleep at the end of the song", 15) { player.index == 2 && !player.isPlaying }
    await wait("sleep timer cleared") { !player.sleepAtEndOfSong }

    // Offline: download, then play from the file.
    library.download(a)
    await wait("download", 60) { library.isDownloaded(a.videoId) }
    let size = library.downloads.first?.bytes ?? 0
    print(size > 3_000_000 ? "ok  " : "FAIL", "downloaded size", size)
    // Its cover is kept with it, for the lock screen offline.
    await wait("cover kept with the download", 10) { Covers.local(a.videoId) != nil }
    await wait("lyrics kept with the download", 20) { KeptLyrics.contains(a.videoId) }
    print(KeptLyrics.get(a.videoId)?.synced == true ? "ok  " : "FAIL", "kept lyrics are synced:",
          KeptLyrics.get(a.videoId)?.lines.count ?? 0, "lines")
    if let cover = Covers.local(a.videoId), let data = try? Data(contentsOf: cover), let image = squareImage(data) {
        print(image.width == image.height && data.count < 120_000 ? "ok  " : "FAIL", "cover", image.width, "px,", data.count, "bytes")
    }
    player.play([a])
    await wait("plays the downloaded file") { player.isPlaying && player.position > 0.5 }
    await wait("its real duration") { abs(player.duration - 230.4) < 1 }
    library.removeDownload(a)
    await wait("download removed") { !library.isDownloaded(a.videoId) }
    await wait("its cover removed") { Covers.local(a.videoId) == nil }
    await wait("its lyrics removed") { !KeptLyrics.contains(a.videoId) }

    // Crossfade: the next song starts 6 s before the end of this one, and
    // takes over once the fade is done (so it is ~6 s in by then).
    player.crossfade = 6
    player.play([a, b])
    await wait("plays (crossfade on)") { player.isPlaying && player.position > 0.5 }
    await wait("next one ready") { player.queue.count > 1 }
    try await Task.sleep(for: .seconds(4))
    player.seek(to: 220)
    await wait("crossfade hands over", 20) { player.index == 1 && player.position > 4 }
    print(player.position < 12 ? "ok  " : "FAIL", "the second song started during the fade, now at",
          String(format: "%.1f", player.position))
    await wait("keeps playing after the fade") { player.isPlaying && player.position > 7 }
    player.togglePlay()
    await wait("pause after a fade") { !player.isPlaying }
    player.crossfade = 0

    // A long song (Pink Floyd's Echoes, 23 min, 23 MB): googlevideo
    // throttles big reads of it to ~32 KB/s, so it starts only because the
    // file comes in ranges; a seek to its middle plays too.
    let long = Track(videoId: "OF4rw3LOfqY", title: "Echoes", artist: "Pink Floyd", durationSecs: 1413)
    let start = Date()
    player.play([long])
    await wait("long song starts", 15) { player.isPlaying && player.position > 0.5 }
    print(Date().timeIntervalSince(start) < 10 ? "ok  " : "FAIL", "long song started in",
          String(format: "%.1f s", Date().timeIntervalSince(start)))
    player.seek(to: 900)
    await wait("long song plays after a seek to 15:00", 15) { player.isPlaying && player.position > 901 }
    player.togglePlay()
}
try await run()
exit(0)
