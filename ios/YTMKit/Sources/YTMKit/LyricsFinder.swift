import Foundation

/// Synced lyrics from LRCLIB, else YouTube Music's plain text. Port of the
/// desktop player's `lyrics.rs`.
public struct LyricsFinder: Sendable {
    static let lrclib = "https://lrclib.net/api/search"
    /// LRCLIB entries whose length differs more than this are another cut.
    static let durationSlack = 4.0

    let session: URLSession
    let catalog: Catalog

    public init(catalog: Catalog, session: URLSession = .shared) {
        self.catalog = catalog
        self.session = session
    }

    public func find(_ track: Track, duration: TimeInterval?) async throws -> Lyrics? {
        if let lyrics = try? await lrclib(track, duration: duration), lyrics.synced { return lyrics }
        return try await catalog.lyrics(track.videoId)
    }

    struct Entry: Decodable {
        var duration: Double?
        var syncedLyrics: String?
        var plainLyrics: String?
    }

    func lrclib(_ track: Track, duration: TimeInterval?) async throws -> Lyrics? {
        let (artist, title) = cleanTitle(artist: track.artist, title: track.title)
        var url = URLComponents(string: Self.lrclib)!
        url.queryItems = [URLQueryItem(name: "track_name", value: title),
                          URLQueryItem(name: "artist_name", value: artist)]
        var request = URLRequest(url: url.url!)
        request.setValue("ytm-player (https://github.com/yaroven/YTMusicPlayer)", forHTTPHeaderField: "User-Agent")
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw YTMError.http("LRCLIB", status) }
        let entries = try JSONDecoder().decode([Entry].self, from: data)
        let fits = entries.filter { e in
            guard let duration, let have = e.duration else { return true }
            return abs(duration - have) <= Self.durationSlack
        }
        let lines: [LyricLine]
        if let lrc = fits.lazy.compactMap(\.syncedLyrics).first(where: { !$0.isEmpty }) {
            lines = parseLrc(lrc)
        } else if let plain = fits.lazy.compactMap(\.plainLyrics).first {
            lines = plain.split(separator: "\n", omittingEmptySubsequences: false)
                .map { LyricLine(at: nil, text: String($0)) }
        } else {
            return nil
        }
        return lines.isEmpty ? nil : Lyrics(lines: lines, source: "LRCLIB")
    }
}

func lrcTime<S: StringProtocol>(_ stamp: S) -> TimeInterval? {
    let parts = stamp.split(separator: ":")
    guard parts.count == 2, let min = Double(parts[0]), let sec = Double(parts[1]) else { return nil }
    return min * 60 + sec
}

/// `[mm:ss.xx] text` lines; untimed ones (metadata tags) are dropped.
/// Enhanced LRC word stamps (`<mm:ss.xx>word`) become word times.
func parseLrc(_ lrc: String) -> [LyricLine] {
    lrc.split(separator: "\n", omittingEmptySubsequences: false).compactMap { line in
        guard line.hasPrefix("["), let close = line.firstIndex(of: "]"),
              let at = lrcTime(line[line.index(after: line.startIndex)..<close]) else { return nil }
        var text = ""
        var times: [TimeInterval] = []
        for (i, part) in line[line.index(after: close)...].split(separator: "<", omittingEmptySubsequences: false).enumerated() {
            if i > 0, let end = part.firstIndex(of: ">"), let when = lrcTime(part[..<end]) {
                let word = part[part.index(after: end)...]
                if !word.trimmingCharacters(in: .whitespaces).isEmpty { times.append(when) }
                text += word
            } else {
                text += (i > 0 ? "<" : "") + part
            }
        }
        return LyricLine(at: at, text: text.trimmingCharacters(in: .whitespaces), wordTimes: times)
    }
}

/// "ABBA - Dancing Queen (Official Music Video)" by "ABBA" → ("ABBA", "Dancing Queen").
func cleanTitle(artist: String, title: String) -> (String, String) {
    var artist = artist.trimmingCharacters(in: .whitespaces)
    var title = title.trimmingCharacters(in: .whitespaces)
    if let dash = title.range(of: " - ") {
        let left = String(title[..<dash.lowerBound])
        if artist.isEmpty || left.lowercased().contains(artist.lowercased()) || artist.hasSuffix("VEVO") {
            artist = left.trimmingCharacters(in: .whitespaces)
            title = String(title[dash.upperBound...]).trimmingCharacters(in: .whitespaces)
        }
    }
    // Drop bracketed decorations: (Official Video), [Lyrics], (Remastered 2009)…
    while true {
        let cuts: [Range<String.Index>] = [("(", ")"), ("[", "]")].compactMap { open, close in
            guard let start = title.range(of: open),
                  let end = title[start.upperBound...].range(of: close) else { return nil }
            return start.lowerBound..<end.upperBound
        }
        guard let cut = cuts.min(by: { $0.lowerBound < $1.lowerBound }) else { break }
        title.removeSubrange(cut)
    }
    for noise in ["ft.", "feat."] {
        if let i = title.lowercased().range(of: noise) {
            let offset = title.lowercased().distance(from: title.lowercased().startIndex, to: i.lowerBound)
            title = String(title.prefix(offset))
        }
    }
    return (artist.trimmingCharacters(in: .whitespaces), title.trimmingCharacters(in: .whitespaces))
}
