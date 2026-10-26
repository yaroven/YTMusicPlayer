import Foundation

/// A song and its clip (YouTube Music's "Song" / "Video"); see
/// `Catalog.mediaPair`. Same rules as the desktop player's `MediaPair`.
public struct MediaPair: Hashable, Sendable {
    /// What plays as sound: the studio song (the clip's own sound only when
    /// no studio song was found).
    public var song: String
    /// The clip "Video" shows, when there is one.
    public var video: String?
    /// Seconds the clip runs ahead of the song: clip time = song time +
    /// offset. Guessed from the lengths (a longer clip has an intro), so it
    /// can be a few seconds off when the extra is an outro instead.
    public var offset: TimeInterval
    /// The song's square album art (googleusercontent), when YouTube's queue
    /// entry carries one; a clip's own picture (i.ytimg.com) is not kept.
    public var cover: String? = nil
}

/// The largest of the `thumbnails` under `value`, when it is square album
/// art (googleusercontent; i.ytimg.com frames are a clip's picture).
func albumArt(_ value: Any) -> String? {
    guard let thumbs = findFirst(value, "thumbnails") as? [JSON],
          let url = thumbs.last?["url"] as? String, url.contains("googleusercontent.com") else { return nil }
    return url
}

/// YouTube Music's search filter for videos (clips).
let searchVideosParams = "EgWKAQIQAWoMEA4QChADEAQQCRAF"

extension Catalog {
    /// The song and the clip of the same recording. YouTube gives only its
    /// own apps the exact pair and time map, so the other half is found by
    /// search: the studio song for a clip, the official clip for a song,
    /// matched by title, artist and length. Without `findClip` a song costs
    /// one request (what it is) and its clip isn't looked for.
    public func mediaPair(videoId: String, title: String, artist: String, findClip: Bool) async throws -> MediaPair {
        let data = try await call("next", ["videoId": videoId, "isAudioOnly": true], english: true)
        let entry = findAll(data, "playlistPanelVideoRenderer")
            .compactMap { $0 as? JSON }
            .first { ($0["videoId"] as? String) == videoId }
        let kind = (entry.flatMap { findFirst($0, "musicVideoType") } as? String)
            ?? (findFirst((data as? JSON)?["currentVideoEndpoint"] ?? [:], "musicVideoType") as? String) ?? ""
        let length = (entry?["lengthText"]).flatMap { parseDuration(text($0)) }.map(Double.init)
        let isSong = kind == "MUSIC_VIDEO_TYPE_ATV" || kind == "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK"
        let cover = entry.flatMap { albumArt($0) }
        if isSong && !findClip { return MediaPair(song: videoId, video: nil, offset: 0, cover: cover) }
        let lead = leadArtist(artist)
        let query = "\(lead) \(songTitle(title, artist: lead))"
        let params = isSong ? searchVideosParams : SearchKind.songs.params
        let rows: [(Track, String?)] = ((try? await call("search", ["query": query, "params": params])).map { data in
            findAll(data, "musicResponsiveListItemRenderer").compactMap { row -> (Track, String?)? in
                guard let r = row as? JSON, let track = listItem(r)?.track else { return nil }
                return (track, findFirst(r, "musicVideoType") as? String)
            }
        }) ?? []
        let other = rows.first { track, type in
            guard track.videoId != videoId, !isSong || type == "MUSIC_VIDEO_TYPE_OMV",
                  sameRecording(title, artist, track.title, track.artist) else { return false }
            guard let mine = length, let theirs = track.durationSecs.map(Double.init) else { return true }
            // A clip runs as long as its song or longer, not much shorter.
            return isSong ? theirs >= mine - 3 && theirs <= mine + 180 : theirs <= mine + 3 && theirs >= mine * 0.6
        }?.0
        guard let other else {
            return MediaPair(song: videoId, video: isSong ? nil : videoId, offset: 0, cover: cover)
        }
        let theirs = other.durationSecs.map(Double.init)
        return MediaPair(song: isSong ? videoId : other.videoId,
                         video: isSong ? other.videoId : videoId,
                         offset: clipOffset(song: isSong ? length : theirs, video: isSong ? theirs : length),
                         cover: cover)
    }
}

/// A longer clip is taken to have an intro of the difference (up to 45 s;
/// beyond that it's more likely scenes after the song).
func clipOffset(song: Double?, video: Double?) -> Double {
    guard let song, let video, video > song, video - song <= 45 else { return 0 }
    return video - song
}

/// Lower case, letters and digits only, words single-spaced.
func normalized(_ s: String) -> String {
    String(s.lowercased().map { $0.isLetter || $0.isNumber ? $0 : " " })
        .split(separator: " ").joined(separator: " ")
}

/// A title without bracketed parts ("(Official Video)", "[4K]") and
/// "feat. …", normalized.
func bareTitle(_ title: String) -> String {
    var out = ""
    var depth = 0
    for c in title {
        switch c {
        case "(", "[": depth += 1
        case ")", "]": depth = max(0, depth - 1)
        default: if depth == 0 { out.append(c) }
        }
    }
    var lower = out.lowercased()
    for noise in [" feat.", " feat ", " ft.", " ft "] {
        if let r = lower.range(of: noise) { lower = String(lower[..<r.lowerBound]) }
    }
    return normalized(lower)
}

/// An artist's lead name: the first of several, without "VEVO" / "Official".
func leadArtist(_ artist: String) -> String {
    let first = artist.split(whereSeparator: { $0 == "," || $0 == "&" }).first.map(String.init) ?? artist
    var name = normalized(first).replacingOccurrences(of: "vevo", with: "").trimmingCharacters(in: .whitespaces)
    for tail in [" official", " music", " topic"] where name.hasSuffix(tail) {
        name = String(name.dropLast(tail.count))
    }
    return name.trimmingCharacters(in: .whitespaces)
}

func related(_ a: String, _ b: String) -> Bool {
    !a.isEmpty && !b.isEmpty && (a.contains(b) || b.contains(a))
}

/// A clip's title without the artist in it ("Artist - Title", "Title –
/// Artist") or decorations; `artist` is the lead artist.
func songTitle(_ title: String, artist: String) -> String {
    let dashed = title.replacingOccurrences(of: "–", with: "-").replacingOccurrences(of: "—", with: "-")
    if let range = dashed.range(of: " - ") {
        let left = String(dashed[..<range.lowerBound]), right = String(dashed[range.upperBound...])
        if related(leadArtist(left), artist) { return bareTitle(right) }
        if related(leadArtist(bareTitle(right)), artist) { return bareTitle(left) }
    }
    return bareTitle(dashed)
}

/// The same title by the same artist, decorations aside.
func sameRecording(_ title: String, _ artist: String, _ otherTitle: String, _ otherArtist: String) -> Bool {
    let x = leadArtist(artist), y = leadArtist(otherArtist)
    let a = songTitle(title, artist: x), b = songTitle(otherTitle, artist: x)
    return !a.isEmpty && a == b && related(x, y)
}
