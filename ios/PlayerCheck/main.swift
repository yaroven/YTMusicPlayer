import Foundation
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
}
try await run()
exit(0)
