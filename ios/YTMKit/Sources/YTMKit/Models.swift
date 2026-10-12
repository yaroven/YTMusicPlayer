import Foundation

/// A song.
public struct Track: Codable, Hashable, Identifiable, Sendable {
    public var videoId: String
    public var title: String
    public var artist: String
    public var durationSecs: Int?

    public var id: String { videoId }

    public init(videoId: String, title: String, artist: String, durationSecs: Int? = nil) {
        self.videoId = videoId
        self.title = title
        self.artist = artist
        self.durationSecs = durationSecs
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

    public var id: String { "\(kind.rawValue):\(browseId)" }

    public init(kind: ItemKind, browseId: String, title: String, subtitle: String,
                thumbnail: URL? = nil, track: Track? = nil) {
        self.kind = kind
        self.browseId = browseId
        self.title = title
        self.subtitle = subtitle
        self.thumbnail = thumbnail
        self.track = track
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
}
