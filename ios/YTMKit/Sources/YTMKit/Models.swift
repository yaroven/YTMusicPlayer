import Foundation

/// A song.
public struct Track: Codable, Hashable, Identifiable, Sendable {
    public var videoId: String
    public var title: String
    public var artist: String
    public var durationSecs: Int?
    /// YouTube Music's "E" badge: explicit lyrics (18+).
    public var explicit = false

    public var id: String { videoId }

    public init(videoId: String, title: String, artist: String, durationSecs: Int? = nil,
                explicit: Bool = false) {
        self.videoId = videoId
        self.title = title
        self.artist = artist
        self.durationSecs = durationSecs
        self.explicit = explicit
    }

    /// Square-cropped by the views; YouTube's 16:9 thumbnail of the video.
    public var thumbnailURL: URL? {
        URL(string: "https://i.ytimg.com/vi/\(videoId)/mqdefault.jpg")
    }

    public var largeThumbnailURL: URL? {
        URL(string: "https://i.ytimg.com/vi/\(videoId)/hqdefault.jpg")
    }
}

/// "ABBA - Topic" (auto-generated channels) → "ABBA".
func cleanArtist(_ channel: String) -> String {
    channel.hasSuffix(" - Topic") ? String(channel.dropLast(" - Topic".count)) : channel
}

public enum SearchKind: String, CaseIterable, Sendable, Identifiable {
    case songs = "Songs"
    case albums = "Albums"
    case artists = "Artists"
    case playlists = "Playlists"

    public var id: String { rawValue }

    /// Protobuf params the web client sends for each tab.
    var params: String {
        switch self {
        case .songs: "EgWKAQIIAWoMEA4QChADEAQQCRAF"
        case .albums: "EgWKAQIYAWoMEA4QChADEAQQCRAF"
        case .artists: "EgWKAQIgAWoMEA4QChADEAQQCRAF"
        case .playlists: "EgWKAQIoAWoMEA4QChADEAQQCRAF"
        }
    }
}

public enum ItemKind: String, Codable, Sendable {
    case song, album, artist, playlist
}

/// A search result or a card on a page.
public struct Item: Hashable, Identifiable, Sendable {
    public var kind: ItemKind
    /// Video id for songs, browse id (`MPREb_…`, `UC…`, `VL…`) otherwise.
    public var browseId: String
    public var title: String
    /// "Album • ABBA • 1992", "Artist • 290M monthly audience", …
    public var subtitle: String
    public var thumbnail: URL?
    /// Songs only.
    public var track: Track?
    /// YouTube Music's "E" badge: explicit lyrics (18+).
    public var explicit = false

    public var id: String { "\(kind.rawValue):\(browseId)" }

    public init(kind: ItemKind, browseId: String, title: String, subtitle: String,
                thumbnail: URL? = nil, track: Track? = nil, explicit: Bool = false) {
        self.kind = kind
        self.browseId = browseId
        self.title = title
        self.subtitle = subtitle
        self.thumbnail = thumbnail
        self.track = track
        self.explicit = explicit
    }
}

/// A titled row of cards.
public struct Shelf: Hashable, Identifiable, Sendable {
    public var title: String
    public var items: [Item]
    public var id: String { title + (items.first?.id ?? "") }

    public init(title: String, items: [Item]) {
        self.title = title
        self.items = items
    }
}

/// An album, artist or playlist page.
public struct Page: Sendable {
    public var kind: ItemKind
    public var id: String
    public var title: String
    public var subtitle: String
    public var thumbnail: URL?
    public var tracks: [Track]
    public var shelves: [Shelf]
}

/// One lyrics line; `at` is when it starts (synced lyrics only).
public struct LyricLine: Hashable, Sendable {
    public var at: TimeInterval?
    public var text: String
    /// Word start times when the lyrics have them (Enhanced LRC).
    public var wordTimes: [TimeInterval] = []

    public init(at: TimeInterval?, text: String, wordTimes: [TimeInterval] = []) {
        self.at = at
        self.text = text
        self.wordTimes = wordTimes
    }

    /// No words: an instrumental break ("", "♪").
    public var instrumental: Bool {
        text.allSatisfy { $0.isWhitespace || "♪♫♬…•".contains($0) }
    }

    /// The words, each with its trailing space.
    public var words: [String] {
        var out: [String] = []
        var current = ""
        var inSpace = false
        for c in text {
            if c.isWhitespace {
                inSpace = true
            } else if inSpace {
                out.append(current)
                current = ""
                inSpace = false
            }
            current.append(c)
        }
        if !current.isEmpty { out.append(current) }
        return out
    }
}

/// Karaoke state: the line being sung and how many words (or, on an
/// instrumental line, dots 0...3) are lit. Line -1 is the intro.
public struct LyricsProgress: Equatable, Sendable {
    public var line: Int
    public var words: Int
    /// Characters lit: the sung words and a share of the word being sung,
    /// so it fills letter by letter.
    public var chars: Int

    public init(line: Int, words: Int, chars: Int? = nil) {
        self.line = line
        self.words = words
        self.chars = chars ?? words
    }
}

public struct Lyrics: Sendable {
    public var lines: [LyricLine]
    /// "LRCLIB", or YouTube Music's credit line.
    public var source: String

    public var synced: Bool { lines.first?.at != nil }

    /// Index of the line sung at `position` (synced lyrics).
    public func currentLine(at position: TimeInterval) -> Int? {
        guard synced else { return nil }
        return lines.lastIndex { ($0.at ?? .infinity) <= position }
    }

    /// Before the first line, a gap this long shows as an instrumental intro.
    public static let introGap: TimeInterval = 4
    static let perChar: TimeInterval = 0.085
    static let lineExtra: TimeInterval = 0.4

    public var hasIntro: Bool {
        synced && (lines.first?.at ?? 0) >= Self.introGap
    }

    /// When each word of line `i` starts: the lyrics' own times, else
    /// spread by length over the time the line is sung (same as the
    /// desktop player's `Lyrics::word_times`).
    public func wordTimes(_ i: Int) -> [TimeInterval] {
        guard lines.indices.contains(i) else { return [] }
        let line = lines[i]
        let words = line.words
        if line.wordTimes.count == words.count, !words.isEmpty { return line.wordTimes }
        guard let start = line.at else { return [] }
        let chars = words.map { max(1, $0.trimmingCharacters(in: .whitespaces).count) }
        let total = chars.reduce(0, +)
        var span = Self.perChar * Double(total) + Self.lineExtra
        if i + 1 < lines.count, let next = lines[i + 1].at {
            let gap = next - start
            span = max(min(span, gap - 0.2), min(gap, 0.5))
        }
        var before = 0
        return chars.map { c in
            defer { before += c }
            return start + span * Double(before) / Double(max(total, 1))
        }
    }

    public func progress(at position: TimeInterval) -> LyricsProgress? {
        guard synced else { return nil }
        if hasIntro, let first = lines.first?.at, position < first {
            return LyricsProgress(line: -1, words: min(3, Int(position / max(first, 0.1) * 4)))
        }
        guard let line = currentLine(at: position) else { return nil }
        let current = lines[line]
        if current.instrumental {
            let start = current.at ?? 0
            let end = line + 1 < lines.count ? (lines[line + 1].at ?? start + 6) : start + 6
            return LyricsProgress(line: line, words: min(3, Int((position - start) / max(end - start, 0.1) * 4)))
        }
        let times = wordTimes(line)
        let words = current.words
        let sung = times.filter { $0 <= position }.count
        var chars = words.prefix(max(0, sung - 1)).reduce(0) { $0 + $1.count }
        if sung > 0, sung <= words.count {
            // The word being sung fills over its time.
            let word = words[sung - 1]
            let letters = word.trimmingCharacters(in: .whitespaces).count
            let start = times[sung - 1]
            let end = sung < times.count ? times[sung] : start + Self.perChar * Double(max(letters, 1)) + Self.lineExtra
            let share = min(1, (position - start) / max(end - start, 0.05))
            chars += share >= 1 ? word.count : max(1, Int((Double(letters) * share).rounded(.up)))
        }
        return LyricsProgress(line: line, words: sung, chars: chars)
    }
}
