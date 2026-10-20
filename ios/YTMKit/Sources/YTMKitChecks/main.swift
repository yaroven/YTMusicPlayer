// Self-checks for YTMKit: `swift run YTMKitChecks` (offline parsing),
// `swift run YTMKitChecks --live` (also YouTube / LRCLIB over the network).
// An executable instead of XCTest: Command Line Tools ship no test framework.
import Foundation
@testable import YTMKit

nonisolated(unsafe) var failures = 0

@MainActor func check(_ condition: @autoclosure () -> Bool, _ what: String, line: Int = #line) {
    if condition() { return }
    failures += 1
    print("FAIL line \(line): \(what)")
}

func json(_ s: String) -> Any {
    try! JSONSerialization.jsonObject(with: Data(s.utf8))
}

func browse(_ id: String, _ type: String) -> String {
    #"{"browseEndpoint":{"browseId":"\#(id)","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"\#(type)"}}}}"#
}

func column(_ runs: String) -> String {
    #"{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":\#(runs)}}}"#
}

let songRow = #"""
{"musicResponsiveListItemRenderer":{
  "flexColumns":[\#(column(#"[{"text":"Dancing Queen"}]"#)),
    \#(column(#"[{"text":"ABBA","navigationEndpoint":\#(browse("UCabba", "MUSIC_PAGE_TYPE_ARTIST"))},{"text":" • "},{"text":"Arrival","navigationEndpoint":\#(browse("MPREb_x", "MUSIC_PAGE_TYPE_ALBUM"))},{"text":" • "},{"text":"3:52"}]"#))],
  "playlistItemData":{"videoId":"YkLLcIKhJ64"},
  "thumbnail":{"musicThumbnailRenderer":{"thumbnail":{"thumbnails":[{"url":"https://a/small","width":60},{"url":"https://a/big","width":226}]}}}}}
"""#

let albumRow = #"""
{"musicResponsiveListItemRenderer":{
  "flexColumns":[\#(column(#"[{"text":"ABBA Gold"}]"#)),
    \#(column(#"[{"text":"Album"},{"text":" • "},{"text":"ABBA"},{"text":" • "},{"text":"1992"}]"#))],
  "navigationEndpoint":\#(browse("MPREb_gold", "MUSIC_PAGE_TYPE_ALBUM"))}}
"""#

@MainActor func offline() {
    // Search rows.
    let search = json(#"{"contents":[\#(songRow),\#(albumRow)]}"#)
    let songs = parseSearch(search, kind: .songs)
    check(songs.count == 1, "one song")
    let t = songs.first?.track
    check(t?.title == "Dancing Queen" && t?.artist == "ABBA" && t?.durationSecs == 232, "song row \(String(describing: t))")
    check(songs.first?.thumbnail?.absoluteString == "https://a/big", "thumbnail ≥ 200 px")
    let albums = parseSearch(search, kind: .albums)
    check(albums.count == 1 && albums.first?.kind == .album && albums.first?.browseId == "MPREb_gold", "album row")
    check(albums.first?.subtitle == "Album • ABBA • 1992", "album subtitle")

    // Album page: rows without an artist get the album's.
    let album = json(#"""
    {"header":{"musicResponsiveHeaderRenderer":{"title":{"runs":[{"text":"ABBA Gold"}]},
       "subtitle":{"runs":[{"text":"Album"},{"text":" • "},{"text":"1992"}]},
       "straplineTextOne":{"runs":[{"text":"ABBA"}]}}},
     "contents":{"musicShelfRenderer":{"contents":[{"musicResponsiveListItemRenderer":{
       "flexColumns":[\#(column(#"[{"text":"Mamma Mia"}]"#)),\#(column("[]"))],
       "fixedColumns":[{"musicResponsiveListItemFixedColumnRenderer":{"text":{"runs":[{"text":"3:33"}]}}}],
       "playlistItemData":{"videoId":"unFR0ZXmB0E"}}}]}}}
    """#)
    let page = parsePage(id: "MPREb_gold", album)
    check(page.kind == .album && page.title == "ABBA Gold", "album page")
    check(page.subtitle == "Album • 1992 • ABBA", "album subtitle: \(page.subtitle)")
    check(page.tracks.count == 1 && page.tracks.first?.artist == "ABBA" && page.tracks.first?.durationSecs == 213,
          "album tracks \(page.tracks)")

    // Artist page: top songs, album shelf, video shelf dropped.
    let artist = json(#"""
    {"header":{"musicImmersiveHeaderRenderer":{"title":{"runs":[{"text":"ABBA"}]}}},
     "contents":[{"musicShelfRenderer":{"contents":[\#(songRow)],
         "bottomEndpoint":{"browseEndpoint":{"browseId":"VLOLAK5uy_top"}}}},
       {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Albums"}]}}},
         "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Arrival"}]},"subtitle":{"runs":[{"text":"1976"}]},
           "navigationEndpoint":\#(browse("MPREb_arrival", "MUSIC_PAGE_TYPE_ALBUM"))}}]}},
       {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Videos"}]}}},
         "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Dancing Queen"}]},
           "navigationEndpoint":{"watchEndpoint":{"videoId":"x"}}}}]}}]}
    """#)
    let ap = parsePage(id: "UCabba", artist)
    check(ap.kind == .artist && ap.tracks.count == 1, "artist top songs")
    check(ap.shelves.count == 1 && ap.shelves.first?.title == "Albums", "video carousel dropped")
    check(ap.shelves.first?.items.first?.kind == .album, "album card")
    check(topSongsPlaylist(artist) == "VLOLAK5uy_top", "top songs playlist")

    // Radio, tabs, artist link.
    let next = json(#"""
    {"contents":[{"playlistPanelVideoRenderer":{"videoId":"5mHzaIehRTE",
        "title":{"runs":[{"text":"Lay All Your Love On Me"}]},
        "longBylineText":{"runs":[{"text":"ABBA","navigationEndpoint":{"browseEndpoint":{"browseId":"UCabba"}}},{"text":" • "},{"text":"Super Trouper"}]},
        "lengthText":{"runs":[{"text":"4:35"}]}}},
      {"tabRenderer":{"title":"Lyrics","endpoint":{"browseEndpoint":{"browseId":"MPLYt_x"}}}}]}
    """#)
    let radio = parseRadio(next)
    check(radio.count == 1 && radio.first?.artist == "ABBA" && radio.first?.durationSecs == 275, "radio")
    check(tabBrowseId(next, title: "Lyrics") == "MPLYt_x" && tabBrowseId(next, title: "Related") == nil, "tabs")
    check(artistId(next, videoId: "5mHzaIehRTE") == "UCabba", "artist id")

    // Lyrics and durations.
    let lyr = parseLyricsText(json(#"""
    {"musicDescriptionShelfRenderer":{"description":{"runs":[{"text":"Half-past twelve\nAnd I'm watching"}]},
      "footer":{"runs":[{"text":"Source: Musixmatch"}]}}}
    """#))
    check(lyr?.text.hasPrefix("Half-past") == true && lyr?.source == "Source: Musixmatch", "YT lyrics")
    // The library: playlist rows carry the id a removal needs; the account's
    // playlists skip "Liked Music" and the "New playlist" button; its artist
    // rows point at "MPLA<channel id>".
    let likedRow = songRow.replacingOccurrences(
        of: #""playlistItemData":{"videoId":"YkLLcIKhJ64"}"#,
        with: #""playlistItemData":{"videoId":"YkLLcIKhJ64","playlistSetVideoId":"set-1"}"#)
    let entries = playlistEntriesIn(json(#"{"contents":[\#(likedRow),\#(songRow)]}"#))
    check(entries.count == 2 && entries[0].setVideoId == "set-1" && entries[1].setVideoId == "",
          "playlist set ids \(entries.map(\.setVideoId))")
    check(bare("VLPLroad") == "PLroad" && bare("PLroad") == "PLroad", "bare playlist id")
    func libraryCard(_ title: String, _ id: String, _ type: String) -> String {
        #"{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"\#(title)"}]},"subtitle":{"runs":[{"text":"Playlist"}]},"navigationEndpoint":\#(browse(id, type))}}"#
    }
    let libraryLists = json(#"""
    {"contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"New playlist"}]}}},
      \#(libraryCard("Liked Music", "VLLM", "MUSIC_PAGE_TYPE_PLAYLIST")),
      \#(libraryCard("Road trip", "VLPLroad", "MUSIC_PAGE_TYPE_PLAYLIST")),
      \#(libraryCard("ABBA Gold", "MPREb_gold", "MUSIC_PAGE_TYPE_ALBUM"))]}
    """#)
    let ownLists = libraryCards(libraryLists).filter {
        $0.kind == .playlist && $0.browseId != "VLLM" && $0.browseId != "LM"
    }
    check(ownLists.count == 1 && ownLists.first?.browseId == "VLPLroad", "own playlists \(ownLists.map(\.title))")
    let artistRow = json(#"""
    {"contents":[{"musicResponsiveListItemRenderer":{
      "flexColumns":[\#(column(#"[{"text":"Korn"}]"#)),\#(column(#"[{"text":"Artist"}]"#))],
      "navigationEndpoint":\#(browse("MPLAUCkorn", "MUSIC_PAGE_TYPE_LIBRARY_ARTIST"))}}]}
    """#)
    let libraryArtist = findAll(artistRow, "musicResponsiveListItemRenderer")
        .compactMap { ($0 as? JSON).flatMap { listItem($0) } }.first
    check(libraryArtist?.kind == .artist, "library artist row")

    check(parseDuration("1:02:03") == 3723 && parseDuration("1.7B plays") == nil && parseDuration("42") == nil, "durations")

    check(cleanTitle(artist: "ABBA", title: "ABBA - Dancing Queen (Official Music Video Remastered)") == ("ABBA", "Dancing Queen"), "clean 1")
    check(cleanTitle(artist: "RickAstleyVEVO", title: "Rick Astley - Never Gonna Give You Up [4K]") == ("Rick Astley", "Never Gonna Give You Up"), "clean 2")
    check(cleanTitle(artist: "Daft Punk", title: "Get Lucky feat. Pharrell Williams") == ("Daft Punk", "Get Lucky"), "clean 3")
    check(cleanTitle(artist: "Queen", title: "Bohemian Rhapsody") == ("Queen", "Bohemian Rhapsody"), "clean 4")
    // Songs and their clips (same rules as the desktop player).
    check(sameRecording("Bohemian Rhapsody", "Queen", "Queen - Bohemian Rhapsody (Official Video Remastered)", "Queen Official"), "pair 1")
    check(sameRecording("Numb", "Linkin Park", "Numb [Official Music Video] [4K UPGRADE] – Linkin Park", "Linkin Park"), "pair 2")
    check(!sameRecording("Numb", "Linkin Park", "Numb / Encore", "Linkin Park"), "pair 3")
    check(!sameRecording("Halo", "Beyoncé", "Halo", "Soil"), "pair 4")
    check(clipOffset(song: 355, video: 367) == 12 && clipOffset(song: 200, video: 400) == 0, "clip offset")

    let karaoke = Lyrics(lines: parseLrc("[00:10.00] One two three\n[00:20.00]\n[00:30.00] Last"), source: "LRCLIB")
    // Only lines timed: no guessed word times, the line lights up whole.
    check(karaoke.wordTimes(0).isEmpty, "no guessed word times")
    check(karaoke.progress(at: 10.1)?.words == 3 && karaoke.progress(at: 10.1)?.chars == 13, "whole line at once")
    let kept = (try? JSONEncoder().encode(karaoke)).flatMap { try? JSONDecoder().decode(Lyrics.self, from: $0) }
    check(kept?.lines == karaoke.lines && kept?.source == "LRCLIB", "lyrics survive being kept on disk")
    check(karaoke.lines[1].instrumental && karaoke.progress(at: 28) == LyricsProgress(line: 1, words: 3), "instrumental dots")
    check(karaoke.hasIntro && karaoke.progress(at: 5) == LyricsProgress(line: -1, words: 2), "intro")
    let exact = Lyrics(lines: parseLrc("[00:05.00] <00:05.00>Hello <00:07.00>world"), source: "LRCLIB")
    check(exact.lines[0].text == "Hello world" && exact.lines[0].words == ["Hello ", "world"], "enhanced text")
    check(exact.wordTimes(0) == [5, 7] && exact.progress(at: 6)?.words == 1 && exact.progress(at: 6)?.chars == 6, "enhanced word times")
    let lrc = Lyrics(lines: parseLrc("[ar:ABBA]\n[00:17.37] Ooh, you can dance\n[00:25.18] Having the time\n[01:02.00]"),
                     source: "LRCLIB")
    check(lrc.lines.count == 3 && lrc.synced, "lrc lines")
    check(lrc.currentLine(at: 10) == nil && lrc.currentLine(at: 20) == 0 && lrc.currentLine(at: 30) == 1, "current line")

    // Player responses.
    let ok = json(#"""
    {"playabilityStatus":{"status":"OK"},"streamingData":{"expiresInSeconds":"21540","adaptiveFormats":[
      {"itag":251,"url":"https://v/opus","mimeType":"audio/webm"},
      {"itag":140,"url":"https://v/aac","mimeType":"audio/mp4","contentLength":"3500000","approxDurationMs":"230399","loudnessDb":3.68}]}}
    """#)
    let stream = try? parsePlayer(ok)
    check(stream?.url.absoluteString == "https://v/aac" && stream?.contentLength == 3_500_000 && stream?.loudnessDb == 3.68, "itag 140")
    check((stream?.expires.timeIntervalSinceNow ?? 0) > 21000, "expiry")
    check(stream?.duration == 230.399, "real duration")
    do {
        _ = try parsePlayer(json(#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#))
        check(false, "age restricted must throw")
    } catch {
        check(error.localizedDescription == "Sign in to confirm your age", "reason passed on")
    }
    check(trackLoudness(json(#"{"playerConfig":{"audioConfig":{"trackAbsoluteLoudnessLkfs":-10.0,"loudnessTargetLkfs":-14.0}}}"#)) == 4, "track loudness")
    // ios/clients.json (the list the app fetches) must decode and match the built-in one.
    let clients = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .appendingPathComponent("../../../clients.json")
    let remote = (try? Data(contentsOf: clients)).flatMap { try? JSONDecoder().decode([ClientProfile].self, from: $0) }
    check(remote == ClientProfile.builtIn, "ios/clients.json decodes to the built-in clients")
    // The "E" badge and Explore chips.
    var badged = json(#"{"contents":[\#(songRow)]}"#) as! JSON
    var rows = badged["contents"] as! [JSON]
    var row = rows[0]["musicResponsiveListItemRenderer"] as! JSON
    row["badges"] = [["musicInlineBadgeRenderer": ["icon": ["iconType": "MUSIC_EXPLICIT_BADGE"]]]]
    rows[0] = ["musicResponsiveListItemRenderer": row]
    badged["contents"] = rows
    let marked = parseSearch(badged, kind: .songs)
    check(marked.first?.explicit == true && marked.first?.track?.explicit == true, "E badge")
    check(parseSearch(json(#"{"contents":[\#(songRow)]}"#), kind: .songs).first?.explicit == false, "no badge")

    let chips = json(#"""
    {"contents":{"gridRenderer":{"header":{"gridHeaderRenderer":{"title":{"runs":[{"text":"Browse"}]}}},
      "items":[
        {"musicNavigationButtonRenderer":{"buttonText":{"runs":[{"text":"Charts"}]},
          "clickCommand":{"browseEndpoint":{"browseId":"FEmusic_charts"}}}},
        {"musicNavigationButtonRenderer":{"buttonText":{"runs":[{"text":"Chill"}]},
          "clickCommand":{"browseEndpoint":{"browseId":"FEmusic_moods_and_genres_category","params":"ggMPOg1u"}}}}]}}}
    """#)
    let exploreShelves = parseShelves(chips)
    check(exploreShelves.count == 1 && exploreShelves[0].title == "Browse", "explore grid")
    check(exploreShelves[0].items.map(\.browseId) == ["FEmusic_charts", "FEmusic_moods_and_genres_category#ggMPOg1u"],
          "chips keep their params")

    check(parseSuggestions(json(#"""
    {"contents":[{"searchSuggestionRenderer":{"suggestion":{"runs":[{"text":"abba "},{"text":"dancing queen"}]}}},
                 {"searchSuggestionRenderer":{"suggestion":{"runs":[{"text":""}]}}}]}
    """#)) == ["abba dancing queen"], "suggestions")

    // Signed-in requests.
    let cookies = ["SAPISID": "abc", "__Secure-3PAPISID": "abc", "SID": "s", "YSC": "y"]
    check(Auth.isSignedIn(cookies) && !Auth.isSignedIn(["YSC": "y"]), "signed-in cookies")
    let header = authorization(cookies, origin: "https://www.youtube.com", now: Date(timeIntervalSince1970: 1_700_000_000))
    // sha1("1700000000 abc https://www.youtube.com"), SAPISID and 3PAPISID
    let h = "1700000000_27b236f59d4ec583d7530f2c7055d2f9c6aecf92"
    check(header == "SAPISIDHASH \(h) SAPISID3PHASH \(h)", "authorization: \(header)")
    let signedHeaders = Auth(cookies: cookies).headers(origin: "https://music.youtube.com")
    check(signedHeaders["Cookie"] == "SAPISID=abc; SID=s; YSC=y; __Secure-3PAPISID=abc", "cookie header")
    check(Auth().headers(origin: "x").isEmpty, "no headers when signed out")
    do {
        _ = try parsePlayer(json(#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#))
    } catch YTMError.signInRequired {
    } catch {
        check(false, "LOGIN_REQUIRED → signInRequired")
    }
    let hlsOnly = json(#"""
    {"playabilityStatus":{"status":"OK"},"videoDetails":{"lengthSeconds":"201"},
     "streamingData":{"hlsManifestUrl":"https://m/master.m3u8","adaptiveFormats":[{"itag":140,"signatureCipher":"s=x"}]}}
    """#)
    check((try? parsePlayer(hlsOnly)) == nil, "cipher only: no stream")
    let viaHLS = try? parsePlayer(hlsOnly, hls: true)
    check(viaHLS?.hls == true && viaHLS?.duration == 201 && viaHLS?.url.absoluteString == "https://m/master.m3u8", "HLS fallback")
    let master = """
    #EXTM3U
    #EXT-X-MEDIA:URI="https://m/api/manifest/hls_playlist/itag/234/index.m3u8",TYPE=AUDIO,GROUP-ID="234",NAME="Default"
    #EXT-X-MEDIA:URI="https://m/api/manifest/hls_playlist/itag/140/index.m3u8",TYPE=AUDIO,GROUP-ID="140",NAME="Default"
    #EXT-X-STREAM-INF:BANDWIDTH=1,CODECS="avc1",AUDIO="234"
    https://m/video.m3u8
    """
    check(audioPlaylist(master, base: URL(string: "https://m/master.m3u8")!)?.absoluteString
          == "https://m/api/manifest/hls_playlist/itag/140/index.m3u8", "audio playlist from HLS master")
    let grid = json(#"""
    {"gridRenderer":{"items":[
      {"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Liked music"}]},"navigationEndpoint":\#(browse("VLLM", "MUSIC_PAGE_TYPE_PLAYLIST"))}},
      {"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Road trip"}]},"navigationEndpoint":\#(browse("VLPLroad", "MUSIC_PAGE_TYPE_PLAYLIST"))}}]}}
    """#)
    check(libraryCards(grid).map(\.browseId) == ["VLLM", "VLPLroad"], "library grid")

    // Signed-in web clients: player script and muxed fallback.
    check(ChallengeSolver.playerURL(inPage: #"x "jsUrl":"/s/player/ec429320/player_es6_tcc.vflset/en_US/base.js","y""#)?.absoluteString
          == "https://www.youtube.com/s/player/ec429320/player_ias.vflset/en_US/base.js", "player script URL (main variant)")
    check(ChallengeSolver.signatureTimestamp(in: "a,signatureTimestamp:20367,b") == 20367, "signatureTimestamp")
    let muxed = json(#"""
    {"playabilityStatus":{"status":"OK"},"streamingData":{"formats":[{"itag":18,"url":"https://v/18","mimeType":"video/mp4"}],
     "adaptiveFormats":[{"itag":140,"signatureCipher":"s=x&url=https://v/140"}]}}
    """#)
    let fallback = try? parsePlayer(muxed, itags: [140, 18])
    check(fallback?.url.absoluteString == "https://v/18" && fallback?.hasVideo == true, "itag 18 when 140 stays ciphered")

    check(visitorDataIn(#"x "VISITOR_DATA":"Cgt abc%3D%3D","y""#) == "Cgt abc%3D%3D", "visitor data")

    // A playlist page has the section list's continuation (more shelves) as
    // well as its track list's: the track list's is the one to follow.
    let twoTokens = json(#"""
    {"contents":{"sectionListRenderer":{"continuations":[{"nextContinuationData":{"continuation":"shelves"}}],
      "contents":[{"musicPlaylistShelfRenderer":{"contents":[\#(songRow),
        {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"songs"}}}}]}}]}}}
    """#)
    check(continuation(twoTokens) == "songs", "the track list's continuation")

    // Home: mood chips, shelf styles, the strapline.
    let home = json(#"""
    {"contents":[{"chipCloudChipRenderer":{"text":{"runs":[{"text":"Relax"}]},
        "navigationEndpoint":{"browseEndpoint":{"browseId":"FEmusic_home","params":"relax"}}}},
      {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{
          "title":{"runs":[{"text":"Quick picks"}]},"strapline":{"runs":[{"text":"JANE"}]}}},
        "contents":[\#(songRow)]}},
      {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Videos"}]}}},
        "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Clip"}]},"aspectRatio":"MUSIC_TWO_ROW_ITEM_THUMBNAIL_ASPECT_RATIO_RECTANGLE_16_9",
          "navigationEndpoint":{"watchEndpoint":{"videoId":"x"}}}}]}}],
     "continuations":[{"nextContinuationData":{"continuation":"more"}}]}
    """#)
    check(parseHomeChips(home) == [HomeChip(title: "Relax", params: "relax")], "home chips")
    let homeShelves = parseShelves(home)
    check(homeShelves.map(\.style) == [.grid, .wide] && homeShelves[0].strapline == "JANE", "shelf styles \(homeShelves.map(\.style))")
    check(homeContinuation(home) == "more", "home continuation")
}

@MainActor func live() async throws {
    let catalog = Catalog()
    let clipPair = try await catalog.mediaPair(videoId: "fJ9rUzIMcZQ", title: "Queen – Bohemian Rhapsody (Official Video Remastered)", artist: "Queen Official", findClip: false)
    check(clipPair.song == "BSTsnWoslP4" && clipPair.video == "fJ9rUzIMcZQ", "live clip plays its studio song \(clipPair)")
    let songPair = try await catalog.mediaPair(videoId: "BSTsnWoslP4", title: "Bohemian Rhapsody", artist: "Queen", findClip: true)
    check(songPair.song == "BSTsnWoslP4" && songPair.video == "fJ9rUzIMcZQ", "live song finds its clip \(songPair)")
    let found = try await catalog.search("abba dancing queen", kind: .songs)
    print("search: \(found.count) songs, first \(found.first?.title ?? "-") by \(found.first?.track?.artist ?? "-")")
    check(!found.isEmpty, "live search")
    let home = try await catalog.home()
    print("home: \(home.count) shelves")
    check(!home.isEmpty, "live home")
    let radio = try await catalog.radio("YkLLcIKhJ64")
    print("radio: \(radio.count) tracks")
    check(radio.count > 10, "live radio")
    if let artist = try await catalog.artistId("YkLLcIKhJ64") {
        let page = try await catalog.page(artist)
        print("artist: \(page.title), \(page.tracks.count) songs, \(page.shelves.count) shelves")
        check(page.tracks.count > 5, "artist's full top songs")
    } else {
        check(false, "live artist id")
    }
    let lyrics = try await LyricsFinder(catalog: catalog)
        .find(Track(videoId: "fJ9rUzIMcZQ", title: "Bohemian Rhapsody", artist: "Queen"), duration: 359)
    print("lyrics: \(lyrics?.lines.count ?? 0) lines from \(lyrics?.source ?? "-"), synced \(lyrics?.synced ?? false)")
    check(lyrics?.synced == true, "live synced lyrics")

    let explore = try await catalog.explore()
    print("explore: \(explore.count) shelves, first \(explore.first?.title ?? "-")")
    check(explore.count > 2, "live explore")
    if let chip = explore.flatMap(\.items).first(where: { $0.browseId.hasPrefix("FEmusic_") }) {
        let page = try await catalog.page(chip.browseId)
        print("chip \(chip.title): \(page.shelves.count) shelves")
        check(!page.shelves.isEmpty, "chip opens as a page")
    } else {
        check(false, "an Explore chip")
    }
    let suggested = try await catalog.suggestions("abba danc")
    print("suggestions: \(suggested.prefix(3))")
    check(suggested.contains { $0.contains("dancing") }, "live suggestions")
    let explicitSongs = try await catalog.search("eminem", kind: .songs)
    check(explicitSongs.contains(where: \.explicit), "live E badges")

    // Home in Ukrainian, with its chips and more shelves behind the first.
    let feed = try await Catalog(language: "uk").homeFeed()
    print("home feed: \(feed.chips.prefix(3).map(\.title)), \(feed.shelves.map(\.title))")
    check(!feed.chips.isEmpty, "home chips live")
    if let token = feed.continuation {
        let more = try await Catalog(language: "uk").moreHome(token, visitor: feed.visitor)
        print("more home: \(more.shelves.map(\.title))")
        check(!more.shelves.isEmpty, "more home shelves")
    } else {
        check(false, "home continuation live")
    }

    // Long playlists: every song, and the first answer on its own (fast).
    let long = try await catalog.page("VLPL0GvsLQil0MmYC96KEs_7dTNsLm1PS6JX")
    let quick = try await catalog.page("VLPL0GvsLQil0MmYC96KEs_7dTNsLm1PS6JX", full: false)
    print("long playlist: \(long.tracks.count) songs, first answer \(quick.tracks.count)")
    check(long.tracks.count > 100, "a playlist past 100 songs")
    check(quick.tracks.count <= 100 && quick.continuation != nil, "first answer + continuation")

    let resolver = Resolver()
    let start = Date()
    let stream = try await resolver.stream("YkLLcIKhJ64")
    print(String(format: "stream: %.1f s, %d bytes, %.1f s long, loudness %@", Date().timeIntervalSince(start),
                 stream.contentLength ?? -1, stream.duration ?? -1, stream.loudnessDb.map { "\($0)" } ?? "-"))
    check(stream.duration != nil, "live duration")
    // Every range must work (a PO-token-bound URL fails past the first MB).
    let length = stream.contentLength ?? 1_000_000
    for offset in [0, length / 2, max(0, length - 70_000)] {
        var request = URLRequest(url: stream.url)
        request.setValue("bytes=\(offset)-\(offset + 65_535)", forHTTPHeaderField: "Range")
        request.setValue("AppleCoreMedia/1.0.0.23A341 (iPhone; U; CPU OS 26_0 like Mac OS X; en_us)",
                         forHTTPHeaderField: "User-Agent")
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        print("  range \(offset): \(status), \(data.count) bytes")
        check(status == 206, "range \(offset)")
    }
}

if CommandLine.arguments.contains("--write-clients") {
    // Regenerates ios/clients.json from ClientProfile.builtIn.
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../../clients.json")
    try! (encoder.encode(ClientProfile.builtIn) + Data("\n".utf8)).write(to: file)
    print("wrote \(file.standardized.path)")
}
offline()
if CommandLine.arguments.contains("--live") {
    do { try await live() } catch {
        failures += 1
        print("FAIL live: \(error)")
    }
}
print(failures == 0 ? "all checks passed" : "\(failures) failed")
exit(failures == 0 ? 0 : 1)
