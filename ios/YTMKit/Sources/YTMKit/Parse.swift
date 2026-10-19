import Foundation

// Ported from the desktop player's `catalog.rs`: lenient parsing, unknown
// renderers are skipped, never an error.

func pageType(_ endpoint: Any?) -> String? {
    string(endpoint, "browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
}

func kindOf(_ pageType: String?) -> ItemKind? {
    switch pageType {
    case "MUSIC_PAGE_TYPE_ALBUM", "MUSIC_PAGE_TYPE_AUDIOBOOK": .album
    // The library's own rows link to "MPLA<channel id>" pages.
    case "MUSIC_PAGE_TYPE_ARTIST", "MUSIC_PAGE_TYPE_USER_CHANNEL", "MUSIC_PAGE_TYPE_LIBRARY_ARTIST": .artist
    case "MUSIC_PAGE_TYPE_PLAYLIST": .playlist
    default: nil
    }
}

/// Playlist ids travel as `PL…`; browse ids carry a `VL` prefix.
func bare(_ playlistId: String) -> String {
    playlistId.hasPrefix("VL") ? String(playlistId.dropFirst(2)) : playlistId
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

/// YouTube Music's "E" badge anywhere below `value`.
func explicitBadge(_ value: Any) -> Bool {
    findAll(value, "iconType").contains { ($0 as? String) == "MUSIC_EXPLICIT_BADGE" }
}

/// `musicNavigationButtonRenderer`: an Explore chip ("Charts", "Chill"),
/// which opens a browse page. Mood categories need `params`, kept after a
/// `#` in the id — browse ids never contain one (`Catalog.page` splits it).
func navButton(_ r: JSON) -> Item? {
    let title = text(r["buttonText"])
    guard let endpoint = pointer(r, "clickCommand/browseEndpoint") as? JSON,
          let id = endpoint["browseId"] as? String, !title.isEmpty else { return nil }
    let full = (endpoint["params"] as? String).map { "\(id)#\($0)" } ?? id
    return Item(kind: .playlist, browseId: full, title: title, subtitle: "")
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
        return Item(kind: kind, browseId: id, title: title, subtitle: subtitle, thumbnail: art,
                    explicit: explicitBadge(r))
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
    let explicit = explicitBadge(r)
    let track = Track(videoId: videoId, title: title, artist: cleanArtist(artist),
                      durationSecs: duration, explicit: explicit)
    return Item(kind: .song, browseId: videoId, title: title, subtitle: subtitle, thumbnail: art,
                track: track, explicit: explicit)
}

/// `musicTwoRowItemRenderer`: a card (album, playlist, artist, song).
func card(_ r: JSON) -> Item? {
    let title = text(r["title"])
    let subtitle = text(r["subtitle"])
    guard let endpoint = r["navigationEndpoint"] as? JSON else { return nil }
    let art = thumbnail(r["thumbnailRenderer"])
    let explicit = explicitBadge(r)
    if let videoId = string(endpoint, "watchEndpoint/videoId") {
        let artist = runs(r["subtitle"]).first { $0["navigationEndpoint"] != nil }?["text"] as? String ?? ""
        let track = Track(videoId: videoId, title: title, artist: cleanArtist(artist), explicit: explicit)
        return Item(kind: .song, browseId: videoId, title: title, subtitle: subtitle, thumbnail: art,
                    track: track, explicit: explicit)
    }
    guard let kind = kindOf(pageType(endpoint)), let id = string(endpoint, "browseEndpoint/browseId") else {
        return nil
    }
    return Item(kind: kind, browseId: id, title: title, subtitle: subtitle, thumbnail: art,
                explicit: explicit)
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
/// Playlist rows as `(song, setVideoId)`; the set id is what removing that
/// entry from the playlist needs, and is empty when YouTube doesn't give one
/// (search results, other people's playlists).
func playlistEntriesIn(_ data: Any) -> [(track: Track, setVideoId: String)] {
    findAll(data, "musicResponsiveListItemRenderer").compactMap { row in
        guard let row = row as? JSON, let track = listItem(row)?.track else { return nil }
        return (track, string(row, "playlistItemData/playlistSetVideoId") ?? "")
    }
}

func songsIn(_ data: Any, albumArtist: String? = nil) -> [Track] {
    findAll(data, "musicResponsiveListItemRenderer")
        .compactMap { ($0 as? JSON).flatMap { listItem($0, albumArtist: albumArtist)?.track } }
}

/// Carousels of cards (and of song rows) with their titles, then the grids
/// Explore and "moods & genres" use instead.
func parseShelves(_ data: Any) -> [Shelf] {
    let carousels = findAll(data, "musicCarouselShelfRenderer").compactMap { shelf -> Shelf? in
        let header = pointer(shelf, "header/musicCarouselShelfBasicHeaderRenderer")
        let title = text(pointer(header, "title"))
        let contents = (shelf as? JSON)?["contents"] as? [JSON] ?? []
        let items = shelfItems(contents)
        guard !items.isEmpty else { return nil }
        let rows = contents.allSatisfy { $0["musicResponsiveListItemRenderer"] != nil }
        let wide = contents.contains {
            (pointer($0, "musicTwoRowItemRenderer/aspectRatio") as? String ?? "").contains("16_9")
        }
        return Shelf(title: title, items: items, style: rows ? .grid : wide ? .wide : .cards,
                     strapline: text(pointer(header, "strapline")),
                     straplineArt: thumbnail(pointer(header, "thumbnail")))
    }
    let grids = findAll(data, "gridRenderer").compactMap { grid -> Shelf? in
        let title = text(pointer(grid, "header/gridHeaderRenderer/title"))
        let items = shelfItems((grid as? JSON)?["items"] as? [JSON] ?? [])
        return items.isEmpty ? nil : Shelf(title: title, items: items)
    }
    return carousels + grids
}

/// The cards of a carousel or grid, whichever renderer they use.
func shelfItems(_ contents: [JSON]) -> [Item] {
    contents.compactMap { c in
        (c["musicTwoRowItemRenderer"] as? JSON).flatMap(card)
            ?? (c["musicResponsiveListItemRenderer"] as? JSON).flatMap { listItem($0) }
            ?? (c["musicNavigationButtonRenderer"] as? JSON).flatMap(navButton)
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
                thumbnail: thumbnail(header?["thumbnail"]), tracks: tracks, shelves: shelves,
                allSongsId: kind == .artist ? topSongsPlaylist(data) : nil)
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

/// Home's mood chips (each a filtered Home).
func parseHomeChips(_ data: Any) -> [HomeChip] {
    findAll(data, "chipCloudChipRenderer").compactMap { chip in
        let title = text((chip as? JSON)?["text"])
        guard !title.isEmpty, let params = string(chip, "navigationEndpoint/browseEndpoint/params") else { return nil }
        return HomeChip(title: title, params: params)
    }
}

/// More of Home's shelves: the section list's own continuation.
func homeContinuation(_ data: Any) -> String? {
    (findFirst(data, "nextContinuationData") as? JSON)?["continuation"] as? String
}

/// Plain query strings from `music/get_search_suggestions`.
func parseSuggestions(_ data: Any) -> [String] {
    findAll(data, "searchSuggestionRenderer")
        .map { text(($0 as? JSON)?["suggestion"]) }
        .filter { !$0.isEmpty }
}

func parseLyricsText(_ data: Any) -> (text: String, source: String)? {
    guard let shelf = findFirst(data, "musicDescriptionShelfRenderer") as? JSON else { return nil }
    let body = text(shelf["description"])
    return body.isEmpty ? nil : (body, text(shelf["footer"]))
}

/// The next page of a track list. The list's own `continuationCommand`
/// comes first: a playlist page also carries the section list's
/// `nextContinuationData` (more shelves, no songs), and following that one
/// cut every playlist off at 100 songs.
func continuation(_ data: Any) -> String? {
    if let t = (findFirst(data, "continuationCommand") as? JSON)?["token"] as? String { return t }
    return (findFirst(data, "nextContinuationData") as? JSON)?["continuation"] as? String
}

/// The song's first artist page, as linked under the song in the queue.
func artistId(_ next: Any, videoId: String) -> String? {
    let items = findAll(next, "playlistPanelVideoRenderer").compactMap { $0 as? JSON }
    let song = items.first { $0["videoId"] as? String == videoId } ?? items.first
    return runs(song?["longBylineText"])
        .compactMap { string($0, "navigationEndpoint/browseEndpoint/browseId") }
        .first { $0.hasPrefix("UC") }
}

/// Cards in a library grid (playlists, albums), not in carousels.
func libraryCards(_ data: Any) -> [Item] {
    findAll(data, "musicTwoRowItemRenderer").compactMap { ($0 as? JSON).flatMap(card) }
        .filter { $0.kind == .playlist || $0.kind == .album }
}
