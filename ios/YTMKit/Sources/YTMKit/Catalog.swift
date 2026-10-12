import Foundation

public enum YTMError: LocalizedError, Sendable {
    case http(String, Int)
    case unplayable(String)
    case noStream

    public var errorDescription: String? {
        switch self {
        case let .http(what, status): "\(what): HTTP \(status)"
        case let .unplayable(reason): reason
        case .noStream: "YouTube returned no audio stream for this song"
        }
    }
}

/// YouTube Music's own API (InnerTube, WEB_REMIX client): quota-free, no
/// sign-in. Mirrors the desktop player's `Catalog`.
public final class Catalog: Sendable {
    static let base = "https://music.youtube.com/youtubei/v1"
    static let clientVersion = "1.20260901.01.00"
    static let maxPages = 10

    let session: URLSession

    public init(session: URLSession = .shared) {
        self.session = session
    }

    func call(_ endpoint: String, _ body: [String: Any]) async throws -> Any {
        var body = body
        body["context"] = ["client": ["clientName": "WEB_REMIX", "clientVersion": Self.clientVersion,
                                      "hl": "en", "gl": "US"]]
        var request = URLRequest(url: URL(string: "\(Self.base)/\(endpoint)?prettyPrint=false")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("https://music.youtube.com", forHTTPHeaderField: "Origin")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else { throw YTMError.http("YouTube Music \(endpoint)", status) }
        return try JSONSerialization.jsonObject(with: data)
    }

    public func search(_ query: String, kind: SearchKind) async throws -> [Item] {
        parseSearch(try await call("search", ["query": query, "params": kind.params]), kind: kind)
    }

    /// Album (`MPREb_…`), artist (`UC…`) or playlist (`VL…` / `PL…`) page.
    public func page(_ browseId: String) async throws -> Page {
        let id = ["PL", "OLAK", "RD"].contains { browseId.hasPrefix($0) } ? "VL" + browseId : browseId
        let data = try await call("browse", ["browseId": id])
        var page = parsePage(id: id, data)
        // Long playlists come in pages.
        var token = page.kind == .playlist ? continuation(data) : nil
        var pages = 1
        while let t = token, pages < Self.maxPages {
            pages += 1
            let more = try await call("browse", ["continuation": t])
            let extra = songsIn(more)
            if extra.isEmpty { break }
            page.tracks += extra
            token = continuation(more)
        }
        // An artist page lists 5 top songs; its "Show all" playlist has them all.
        if page.kind == .artist, let all = topSongsPlaylist(data),
           let list = try? await self.page(all), list.tracks.count > page.tracks.count {
            page.tracks = list.tracks
        }
        return page
    }

    /// Home page shelves (not personalised).
    public func home() async throws -> [Shelf] {
        parseShelves(try await call("browse", ["browseId": "FEmusic_home"]))
    }

    /// What YouTube Music plays after the song (≈50 songs, the song first).
    public func radio(_ videoId: String) async throws -> [Track] {
        parseRadio(try await next(videoId))
    }

    public func related(_ videoId: String) async throws -> [Shelf] {
        guard let id = tabBrowseId(try await next(videoId), title: "Related") else { return [] }
        return parseShelves(try await call("browse", ["browseId": id]))
    }

    /// Plain lyrics from YouTube Music's provider.
    public func lyrics(_ videoId: String) async throws -> Lyrics? {
        guard let id = tabBrowseId(try await next(videoId), title: "Lyrics"),
              let found = parseLyricsText(try await call("browse", ["browseId": id])) else { return nil }
        let lines = found.text.split(separator: "\n", omittingEmptySubsequences: false)
            .map { LyricLine(at: nil, text: String($0)) }
        return Lyrics(lines: lines, source: found.source)
    }

    /// The song's (first) artist page id.
    public func artistId(_ videoId: String) async throws -> String? {
        YTMKit.artistId(try await next(videoId), videoId: videoId)
    }

    func next(_ videoId: String) async throws -> Any {
        try await call("next", ["videoId": videoId, "playlistId": "RDAMVM\(videoId)", "isAudioOnly": true])
    }
}
