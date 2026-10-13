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
    check(parseDuration("1:02:03") == 3723 && parseDuration("1.7B plays") == nil && parseDuration("42") == nil, "durations")

    check(cleanTitle(artist: "ABBA", title: "ABBA - Dancing Queen (Official Music Video Remastered)") == ("ABBA", "Dancing Queen"), "clean 1")
    check(cleanTitle(artist: "RickAstleyVEVO", title: "Rick Astley - Never Gonna Give You Up [4K]") == ("Rick Astley", "Never Gonna Give You Up"), "clean 2")
    check(cleanTitle(artist: "Daft Punk", title: "Get Lucky feat. Pharrell Williams") == ("Daft Punk", "Get Lucky"), "clean 3")
    check(cleanTitle(artist: "Queen", title: "Bohemian Rhapsody") == ("Queen", "Bohemian Rhapsody"), "clean 4")

    let karaoke = Lyrics(lines: parseLrc("[00:10.00] One two three\n[00:20.00]\n[00:30.00] Last"), source: "LRCLIB")
    let times = karaoke.wordTimes(0)
    check(times.count == 3 && times[0] == 10 && times[2] > times[1] && times[2] < 12, "word times \(times)")
    check(karaoke.progress(at: 10.1)?.words == 1 && (1...3).contains(karaoke.progress(at: 10.1)?.chars ?? 0), "first word, letter by letter")
    check(karaoke.progress(at: 15)?.words == 3 && karaoke.progress(at: 15)?.chars == 13, "whole line")
    check(karaoke.lines[1].instrumental && karaoke.progress(at: 28) == LyricsProgress(line: 1, words: 3), "instrumental dots")
    check(karaoke.hasIntro && karaoke.progress(at: 5) == LyricsProgress(line: -1, words: 2), "intro")
    let exact = Lyrics(lines: parseLrc("[00:05.00] <00:05.00>Hello <00:07.00>world"), source: "LRCLIB")
    check(exact.lines[0].text == "Hello world" && exact.lines[0].words == ["Hello ", "world"], "enhanced text")
    check(exact.wordTimes(0) == [5, 7] && exact.progress(at: 6)?.words == 1 && exact.progress(at: 6)?.chars == 3, "enhanced word times")
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
}

@MainActor func live() async throws {
    let catalog = Catalog()
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
