import Foundation
import YTMKit

/// Lyrics kept on the phone (Application Support/lyrics/<video id>.json):
/// every downloaded song's, so they show offline, and the last `recent`
/// others, so a song played again doesn't ask LRCLIB again.
enum KeptLyrics {
    static let recent = 50

    static let directory: URL = {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let dir = support.appendingPathComponent("lyrics", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }()

    private static func file(_ videoId: String) -> URL? {
        // Video ids are 11 URL-safe characters; nothing else names a file.
        let valid = videoId.count == 11 && videoId.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") }
        return valid ? directory.appendingPathComponent("\(videoId).json") : nil
    }

    static func get(_ videoId: String) -> Lyrics? {
        guard let file = file(videoId), let data = try? Data(contentsOf: file) else { return nil }
        return try? JSONDecoder().decode(Lyrics.self, from: data)
    }

    static func contains(_ videoId: String) -> Bool {
        file(videoId).map { FileManager.default.fileExists(atPath: $0.path) } ?? false
    }

    /// Saves `lyrics`, then drops the oldest that `keep` (the downloaded
    /// songs) doesn't name, beyond `recent`.
    static func put(_ videoId: String, _ lyrics: Lyrics, keep: Set<String>) {
        guard let file = file(videoId), let data = try? JSONEncoder().encode(lyrics),
              (try? data.write(to: file, options: .atomic)) != nil else { return }
        let names = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        let others = names
            .filter { !keep.contains($0.deletingPathExtension().lastPathComponent) }
            .map { ($0, (try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast) }
            .sorted { $0.1 > $1.1 }
        for (url, _) in others.dropFirst(recent) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    static func remove(_ videoId: String) {
        if let file = file(videoId) { try? FileManager.default.removeItem(at: file) }
    }

    /// Kept lyrics, else found (and kept) — what the lyrics pane shows.
    static func find(_ track: Track, duration: TimeInterval?, keep: Set<String>) async throws -> Lyrics? {
        if let kept = get(track.videoId) { return kept }
        let found = try await Services.lyrics.find(track, duration: duration)
        if let found { put(track.videoId, found, keep: keep) }
        return found
    }
}
