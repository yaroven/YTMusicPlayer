import Foundation

public enum YTMError: LocalizedError, Sendable {
    case http(String, Int)
    case unplayable(String)
    /// Age-restricted (or a bot check): needs a signed-in account.
    case signInRequired(String)
    case noStream

    public var errorDescription: String? {
        switch self {
        case let .http(what, status): "\(what): HTTP \(status)"
        case let .unplayable(reason), let .signInRequired(reason): reason
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
    let auth: Auth?
    /// The language shelves, titles and chips come in ("en", "uk").
    let language: String

    /// With `auth` signed in, requests go as the account (personal home,
    /// its library).
    public init(session: URLSession = .shared, auth: Auth? = nil, language: String = "en") {
        self.session = session
        self.auth = auth
        self.language = language
    }

    /// Whether requests go as an account (its library, its own home).
    public var signedIn: Bool { auth?.signedIn == true }

    /// `english`: the answer is parsed by its English labels (the song's
    /// "Lyrics" / "Related" tabs). `visitor`: continuations of a feed need
    /// the visitor id its first answer came with.
    func call(_ endpoint: String, _ body: [String: Any], english: Bool = false,
              visitor: String? = nil) async throws -> Any {
        var body = body
        let hl = english ? "en" : language
        var client: [String: Any] = ["clientName": "WEB_REMIX", "clientVersion": Self.clientVersion,
                                     "hl": hl, "gl": hl == "uk" ? "UA" : "US"]
        client["visitorData"] = visitor
        body["context"] = ["client": client]
        var request = URLRequest(url: URL(string: "\(Self.base)/\(endpoint)?prettyPrint=false")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("https://music.youtube.com", forHTTPHeaderField: "Origin")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let data: Data
        let response: URLResponse
        if let auth, auth.signedIn {
            for (name, value) in auth.headers(origin: "https://music.youtube.com") {
                request.setValue(value, forHTTPHeaderField: name)
            }
            (data, response) = try await auth.session.data(for: request)
            auth.absorb(response)
        } else {
            (data, response) = try await session.data(for: request)
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else { throw YTMError.http("YouTube Music \(endpoint)", status) }
        return try JSONSerialization.jsonObject(with: data)
    }

    public func search(_ query: String, kind: SearchKind) async throws -> [Item] {
        parseSearch(try await call("search", ["query": query, "params": kind.params]), kind: kind)
    }

    /// Album (`MPREb_…`), artist (`UC…`) or playlist (`VL…` / `PL…`) page.
    /// With `full: false` only the first answer: up to 100 songs, the rest
    /// left in `continuation` (and an artist's 5 top songs, the rest in
    /// `allSongsId`), so a screen can show it at once and add the rest.
    public func page(_ browseId: String, full: Bool = true) async throws -> Page {
        // Explore chips carry their params after a `#` (see `navButton`).
        let parts = browseId.split(separator: "#", maxSplits: 1)
        let browseId = String(parts[0])
        let id = ["PL", "OLAK", "RD"].contains { browseId.hasPrefix($0) } ? "VL" + browseId : browseId
        var body: [String: Any] = ["browseId": id]
        if parts.count > 1 { body["params"] = String(parts[1]) }
        let data = try await call("browse", body)
        var page = parsePage(id: id, data)
        if !full {
            page.continuation = page.kind == .playlist ? continuation(data) : nil
            return page
        }
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

    /// The next part of a long track list, and the token for the one after.
    public func moreTracks(_ token: String) async throws -> (tracks: [Track], next: String?) {
        let more = try await call("browse", ["continuation": token])
        let tracks = songsIn(more)
        return (tracks, tracks.isEmpty ? nil : continuation(more))
    }

    /// Explore: charts, new releases, moods and genres. Its chips open as
    /// pages like any other (`page`).
    public func explore() async throws -> [Shelf] {
        parseShelves(try await call("browse", ["browseId": "FEmusic_explore"]))
    }

    /// What YouTube Music suggests while you type (quota-free, ~100 ms).
    public func suggestions(_ query: String) async throws -> [String] {
        let query = query.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return [] }
        return parseSuggestions(try await call("music/get_search_suggestions", ["input": query]))
    }

    /// Home page shelves (not personalised).
    public func home() async throws -> [Shelf] {
        parseShelves(try await call("browse", ["browseId": "FEmusic_home"]))
    }

    /// Home's first shelves and its mood chips; with `params` (a chip's),
    /// Home for that mood.
    public func homeFeed(params: String? = nil) async throws -> HomeFeed {
        var body: [String: Any] = ["browseId": "FEmusic_home"]
        body["params"] = params
        let data = try await call("browse", body)
        return HomeFeed(chips: parseHomeChips(data), shelves: parseShelves(data),
                        continuation: homeContinuation(data),
                        visitor: string(data, "responseContext/visitorData"))
    }

    /// Home's next shelves (three or so at a time) and the token after.
    public func moreHome(_ token: String, visitor: String?) async throws -> (shelves: [Shelf], next: String?) {
        let data = try await call("browse", ["continuation": token], visitor: visitor)
        let shelves = parseShelves(data)
        return (shelves, shelves.isEmpty ? nil : homeContinuation(data))
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

    /// The signed-in account's name ("Jane Doe"), to show who is signed in.
    public func accountName() async throws -> String? {
        try await accountInfo()?.name
    }

    /// The signed-in account's name and picture.
    public func accountInfo() async throws -> AccountInfo? {
        let data = try await call("account/account_menu", [:])
        let name = text(findFirst(data, "accountName"))
        guard !name.isEmpty else { return nil }
        return AccountInfo(name: name, photo: thumbnail(findFirst(data, "accountPhoto")))
    }

    /// The signed-in account's liked songs (YouTube Music's "Liked music").
    public func likedSongs() async throws -> [Track] {
        try await page("VLLM").tracks
    }

    /// The account's own playlists (not Liked Music, not the "New playlist"
    /// button, which has no page).
    public func libraryPlaylists() async throws -> [Item] {
        let data = try await call("browse", ["browseId": "FEmusic_liked_playlists"])
        return libraryCards(data).filter {
            $0.kind == .playlist && $0.browseId != "VLLM" && $0.browseId != "LM"
        }
    }

    /// Albums the account saved to its library.
    public func libraryAlbums() async throws -> [Item] {
        let data = try await call("browse", ["browseId": "FEmusic_liked_albums"])
        return libraryCards(data).filter { $0.kind == .album }
    }

    /// Artists the account has songs or albums from (what its Artists list
    /// shows). Their ids are channel ids, as everywhere else.
    public func libraryArtists() async throws -> [Item] {
        let data = try await call("browse", ["browseId": "FEmusic_library_corpus_track_artists"])
        return findAll(data, "musicResponsiveListItemRenderer")
            .compactMap { ($0 as? JSON).flatMap { listItem($0) } }
            .filter { $0.kind == .artist }
            .map { item in
                guard item.browseId.hasPrefix("MPLA") else { return item }
                var item = item
                item.browseId = String(item.browseId.dropFirst(4))
                return item
            }
    }

    /// A playlist's songs with each entry's `setVideoId`, which removing it
    /// again needs. Paged like `page`.
    public func playlistEntries(_ playlistId: String) async throws -> [(track: Track, setVideoId: String)] {
        let data = try await call("browse", ["browseId": "VL" + bare(playlistId)])
        var entries = playlistEntriesIn(data)
        var token = continuation(data)
        var pages = 1
        while let t = token, pages < Self.maxPages {
            pages += 1
            let more = try await call("browse", ["continuation": t])
            let extra = playlistEntriesIn(more)
            if extra.isEmpty { break }
            entries += extra
            token = continuation(more)
        }
        return entries
    }

    // MARK: Writes (signed in)
    //
    // The same endpoints the YouTube Music site uses, so the account's
    // library really changes — no Data API, no quota, no OAuth client.

    /// Likes, dislikes or un-rates a song.
    public func rate(_ videoId: String, _ rating: Rating) async throws {
        let endpoint = switch rating {
        case .like: "like/like"
        case .dislike: "like/dislike"
        case .none: "like/removelike"
        }
        _ = try await write(endpoint, ["target": ["videoId": videoId]])
    }

    /// Subscribes to a channel (following an artist) or undoes it.
    public func subscribe(_ channelId: String, on: Bool) async throws {
        _ = try await write(on ? "subscription/subscribe" : "subscription/unsubscribe",
                            ["channelIds": [channelId]])
    }

    /// Creates a private playlist; returns its id (`PL…`).
    public func createPlaylist(_ title: String, videoIds: [String] = []) async throws -> String {
        var body: [String: Any] = ["title": title, "privacyStatus": "PRIVATE"]
        if !videoIds.isEmpty { body["videoIds"] = videoIds }
        let data = try await write("playlist/create", body)
        guard let id = (data as? JSON)?["playlistId"] as? String ?? findFirst(data, "playlistId") as? String else {
            throw YTMError.http("YouTube Music didn't return the new playlist's id", 200)
        }
        return bare(id)
    }

    public func deletePlaylist(_ playlistId: String) async throws {
        _ = try await write("playlist/delete", ["playlistId": bare(playlistId)])
    }

    public func renamePlaylist(_ playlistId: String, to title: String) async throws {
        _ = try await editPlaylist(playlistId, ["action": "ACTION_SET_PLAYLIST_NAME", "playlistName": title])
    }

    /// Adds a song to a playlist; returns its `setVideoId`.
    public func addToPlaylist(_ playlistId: String, _ videoId: String) async throws -> String {
        let data = try await editPlaylist(playlistId, ["action": "ACTION_ADD_VIDEO", "addedVideoId": videoId])
        return findFirst(data, "setVideoId") as? String ?? ""
    }

    /// Moves the entry `setVideoId` in front of `before` (another entry's
    /// set id), or to the end when `before` is nil.
    public func moveInPlaylist(_ playlistId: String, setVideoId: String, before: String?) async throws {
        var action: [String: Any] = ["action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": setVideoId]
        if let before { action["movedSetVideoIdSuccessor"] = before }
        _ = try await editPlaylist(playlistId, action)
    }

    /// Removes one entry from a playlist (`setVideoId` identifies it: the
    /// same song can sit in a playlist more than once).
    public func removeFromPlaylist(_ playlistId: String, _ videoId: String, setVideoId: String) async throws {
        _ = try await editPlaylist(playlistId, ["action": "ACTION_REMOVE_VIDEO",
                                                "removedVideoId": videoId,
                                                "setVideoId": setVideoId])
    }

    private func editPlaylist(_ playlistId: String, _ action: [String: Any]) async throws -> Any {
        try await write("browse/edit_playlist", ["playlistId": bare(playlistId), "actions": [action]])
    }

    /// A write needs the sign-in, and YouTube reports failures in the body
    /// rather than the status code.
    private func write(_ endpoint: String, _ body: [String: Any]) async throws -> Any {
        guard let auth, auth.signedIn else {
            throw YTMError.signInRequired("Sign in with Google to change your library")
        }
        let data = try await call(endpoint, body)
        if let status = (data as? JSON)?["status"] as? String, status != "STATUS_SUCCEEDED" {
            throw YTMError.http("YouTube Music \(endpoint): \(status)", 200)
        }
        return data
    }

    func next(_ videoId: String) async throws -> Any {
        try await call("next", ["videoId": videoId, "playlistId": "RDAMVM\(videoId)", "isAudioOnly": true],
                       english: true)
    }
}
