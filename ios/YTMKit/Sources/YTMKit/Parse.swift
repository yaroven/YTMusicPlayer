import Foundation

// Ported from the desktop player's `catalog.rs`: lenient parsing, unknown
// renderers are skipped, never an error.

func pageType(_ endpoint: Any?) -> String? {
    string(endpoint, "browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
}

func kindOf(_ pageType: String?) -> ItemKind? {
    switch pageType {
    case "MUSIC_PAGE_TYPE_ALBUM", "MUSIC_PAGE_TYPE_AUDIOBOOK": .album
    case "MUSIC_PAGE_TYPE_ARTIST", "MUSIC_PAGE_TYPE_USER_CHANNEL": .artist
    case "MUSIC_PAGE_TYPE_PLAYLIST": .playlist
    default: nil
    }
}

/// "3:52" / "1:02:03" → seconds.
func parseDuration(_ s: String) -> Int? {
    let parts = s.trimmingCharacters(in: .whitespaces).split(separator: ":")
    guard parts.count >= 2 else { return nil }
    var total = 0
    for part in parts {
        guard let n = Int(part) else { return nil }
        total = total * 60 + n
    }
    return total
}

/// The thumbnail ≥ 200 px wide (else the largest).
func thumbnail(_ value: Any?) -> URL? {
    guard let value, let thumbs = findFirst(value, "thumbnails") as? [JSON] else { return nil }
    let pick = thumbs.first { ($0["width"] as? Int ?? 0) >= 200 } ?? thumbs.last
    return (pick?["url"] as? String).flatMap(URL.init(string:))
}

/// `musicResponsiveListItemRenderer`: a row in search results, shelves and
/// track lists. `albumArtist` fills in album pages' artist-less rows.
func listItem(_ r: JSON, albumArtist: String? = nil) -> Item? {
    let columns = r["flexColumns"] as? [Any] ?? []
    func column(_ i: Int) -> Any? {
        columns.indices.contains(i) ? pointer(columns[i], "musicResponsiveListItemFlexColumnRenderer/text") : nil
    }
    let title = text(column(0))
    guard !title.isEmpty else { return nil }
    let second = runs(column(1))
    let subtitle = text(column(1))
    let art = thumbnail(r["thumbnail"])

    let videoId = string(r, "playlistItemData/videoId")
        ?? (findFirst(r, "watchEndpoint") as? JSON)?["videoId"] as? String
    if let endpoint = r["navigationEndpoint"] as? JSON, endpoint["browseEndpoint"] != nil {
        guard let kind = kindOf(pageType(endpoint)),
              let id = string(endpoint, "browseEndpoint/browseId") else { return nil }
        return Item(kind: kind, browseId: id, title: title, subtitle: subtitle, thumbnail: art)
    }
    guard let videoId else { return nil }
    // Artists are the runs linking to artist pages; else the first run.
    let artists = second
        .filter { kindOf(pageType($0["navigationEndpoint"])) == .artist }
        .compactMap { $0["text"] as? String }
    let artist = !artists.isEmpty ? artists.joined(separator: ", ")
        : albumArtist ?? (second.first?["text"] as? String ?? "")
    let fixed = (r["fixedColumns"] as? [Any])?.first
    let duration = parseDuration(text(pointer(fixed, "musicResponsiveListItemFixedColumnRenderer/text")))
        ?? second.reversed().compactMap { ($0["text"] as? String).flatMap(parseDuration) }.first
    let track = Track(videoId: videoId, title: title, artist: cleanArtist(artist), durationSecs: duration)
    return Item(kind: .song, browseId: videoId, title: title, subtitle: subtitle, thumbnail: art, track: track)
}

/// `musicTwoRowItemRenderer`: a card (album, playlist, artist, song).
func card(_ r: JSON) -> Item? {
    let title = text(r["title"])
    let subtitle = text(r["subtitle"])
    guard let endpoint = r["navigationEndpoint"] as? JSON else { return nil }
    let art = thumbnail(r["thumbnailRenderer"])
    if let videoId = string(endpoint, "watchEndpoint/videoId") {
        let artist = runs(r["subtitle"]).first { $0["navigationEndpoint"] != nil }?["text"] as? String ?? ""
        let track = Track(videoId: videoId, title: title, artist: cleanArtist(artist))
        return Item(kind: .song, browseId: videoId, title: title, subtitle: subtitle, thumbnail: art, track: track)
    }
    guard let kind = kindOf(pageType(endpoint)), let id = string(endpoint, "browseEndpoint/browseId") else {
        return nil
    }
    return Item(kind: kind, browseId: id, title: title, subtitle: subtitle, thumbnail: art)
}

func parseSearch(_ data: Any, kind: SearchKind) -> [Item] {
    let want: ItemKind = switch kind {
    case .songs: .song
    case .albums: .album
    case .artists: .artist
    case .playlists: .playlist
    }
    return findAll(data, "musicResponsiveListItemRenderer")
        .compactMap { ($0 as? JSON).flatMap { listItem($0) } }
        .filter { $0.kind == want }
}

/// Songs in list rows (album tracks, top songs, playlist items).
func songsIn(_ data: Any, albumArtist: String? = nil) -> [Track] {
    findAll(data, "musicResponsiveListItemRenderer")
        .compactMap { ($0 as? JSON).flatMap { listItem($0, albumArtist: albumArtist)?.track } }
}

/// Carousels of cards (and of song rows) with their titles.
func parseShelves(_ data: Any) -> [Shelf] {
    findAll(data, "musicCarouselShelfRenderer").compactMap { shelf in
        let title = text(pointer(shelf, "header/musicCarouselShelfBasicHeaderRenderer/title"))
        let contents = (shelf as? JSON)?["contents"] as? [JSON] ?? []
        let items = contents.compactMap { c in
            (c["musicTwoRowItemRenderer"] as? JSON).flatMap(card)
                ?? (c["musicResponsiveListItemRenderer"] as? JSON).flatMap { listItem($0) }
        }
        return items.isEmpty ? nil : Shelf(title: title, items: items)
    }
}

/// The playlist behind an artist page's "Top songs" shelf.
func topSongsPlaylist(_ data: Any) -> String? {
    for shelf in findAll(data, "musicShelfRenderer") {
        let endpoint = (shelf as? JSON)?["bottomEndpoint"] ?? pointer(shelf, "title/runs/0/navigationEndpoint")
        if let id = string(endpoint, "browseEndpoint/browseId"), id.hasPrefix("VL") { return id }
    }
    return nil
}

func parsePage(id: String, _ data: Any) -> Page {
    let kind: ItemKind = id.hasPrefix("MPRE") ? .album : id.hasPrefix("UC") ? .artist : .playlist
    let header = ["musicImmersiveHeaderRenderer", "musicVisualHeaderRenderer",
                  "musicResponsiveHeaderRenderer", "musicDetailHeaderRenderer"]
        .lazy.compactMap { findFirst(data, $0) as? JSON }.first
    let title = text(header?["title"])
    let strapline = text(header?["straplineTextOne"])
    let sub = text(header?["subtitle"])
    let subtitle = kind == .album && !strapline.isEmpty ? "\(sub) • \(strapline)"
        : !sub.isEmpty ? sub : text(header?["monthlyListenerCount"])
    let albumArtist = kind == .album && !strapline.isEmpty ? strapline : nil
    let tracks = ["musicShelfRenderer", "musicPlaylistShelfRenderer"]
        .flatMap { findAll(data, $0) }
        .flatMap { songsIn($0, albumArtist: albumArtist) }
    // Video carousels duplicate the songs; keep albums, artists, lists.
    let shelves = parseShelves(data).filter { $0.items.contains { $0.kind != .song } }
    return Page(kind: kind, id: id, title: title, subtitle: subtitle,
                thumbnail: thumbnail(header?["thumbnail"]), tracks: tracks, shelves: shelves)
}

func parseRadio(_ data: Any) -> [Track] {
    findAll(data, "playlistPanelVideoRenderer").compactMap { value in
        guard let r = value as? JSON, let videoId = r["videoId"] as? String else { return nil }
        let artist = runs(r["longBylineText"]).first?["text"] as? String ?? ""
        return Track(videoId: videoId, title: text(r["title"]), artist: cleanArtist(artist),
                     durationSecs: parseDuration(text(r["lengthText"])))
    }
}

func tabBrowseId(_ next: Any, title: String) -> String? {
    let tab = findAll(next, "tabRenderer").first { ($0 as? JSON)?["title"] as? String == title }
    return string(tab, "endpoint/browseEndpoint/browseId")
}

func parseLyricsText(_ data: Any) -> (text: String, source: String)? {
    guard let shelf = findFirst(data, "musicDescriptionShelfRenderer") as? JSON else { return nil }
    let body = text(shelf["description"])
    return body.isEmpty ? nil : (body, text(shelf["footer"]))
}

func continuation(_ data: Any) -> String? {
    if let c = findFirst(data, "nextContinuationData") as? JSON, let t = c["continuation"] as? String { return t }
    return (findFirst(data, "continuationCommand") as? JSON)?["token"] as? String
}

/// The song's first artist page, as linked under the song in the queue.
func artistId(_ next: Any, videoId: String) -> String? {
    let items = findAll(next, "playlistPanelVideoRenderer").compactMap { $0 as? JSON }
    let song = items.first { $0["videoId"] as? String == videoId } ?? items.first
    return runs(song?["longBylineText"])
        .compactMap { string($0, "navigationEndpoint/browseEndpoint/browseId") }
        .first { $0.hasPrefix("UC") }
}
