import Foundation

/// An InnerTube client the player endpoint is asked as. Clients that hand
/// out plain (unciphered, no PO token) audio URLs change over time, so the
/// list can be replaced without a new build: `ios/clients.json` in the repo.
public struct ClientProfile: Codable, Sendable, Hashable {
    public var name: String
    public var version: String
    /// `X-YouTube-Client-Name`.
    public var id: String
    public var userAgent: String
    public var deviceMake: String?
    public var deviceModel: String?
    public var osName: String?
    public var osVersion: String?
    /// Needs `visitorData` from youtube.com (VISIONOS does: without it the
    /// URLs 403).
    public var visitor: Bool
    /// Sent with the signed-in account's cookies, and only for songs the
    /// anonymous clients can't play (age-restricted).
    public var auth: Bool?
    /// Without a plain audio URL, take the HLS manifest's audio playlist.
    public var hls: Bool?
    /// Web clients: formats are ciphered and throttled; solve YouTube's JS
    /// challenges (Challenges.swift) for `itags`.
    public var cipher: Bool?
    /// Formats to take, in order (default: 140, the AAC audio). 18 is the
    /// muxed 360p MP4: its audio is what plays.
    public var itags: [Int]?

    /// Measured 2026-10-12: itag 140 URL, every range 206, AppleCoreMedia UA
    /// accepted. IOS URLs 403 past the first chunk (PO token); ANDROID_VR
    /// and TVHTML5 hit "confirm you're not a bot".
    public static let visionOS = ClientProfile(
        name: "VISIONOS", version: "1.02", id: "101",
        userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
        deviceMake: "Apple", deviceModel: "RealityDevice17,1",
        osName: "visionOS", osVersion: "26.5.23O471", visitor: true)

    /// Signed-in candidates for age-restricted songs. Untested with a real
    /// account when written: Settings → "Test age-restricted playback"
    /// shows which one works, and `ios/clients.json` reorders them.
    public static let signedIn: [ClientProfile] = {
        var vision = visionOS
        vision.auth = true
        vision.hls = true
        let ios = ClientProfile(
            name: "IOS", version: "21.26.4", id: "5",
            userAgent: "com.google.ios.youtube/21.26.4 (iPhone16,2; U; CPU iOS 18_5_0 like Mac OS X;)",
            deviceMake: "Apple", deviceModel: "iPhone16,2", osName: "iPhone", osVersion: "18.5.0.22F76",
            visitor: true, auth: true, hls: true)
        // Measured 2026-10-12 with a real account: signed in, VISIONOS and
        // IOS answer HTTP 400; MWEB returns OK with every format ciphered,
        // and yt-dlp plays its itag 18 (other itags want a PO token).
        let mweb = ClientProfile(
            name: "MWEB", version: "2.20260708.05.00", id: "2",
            userAgent: "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1",
            visitor: true, auth: true, cipher: true, itags: [140, 18])
        let safari = ClientProfile(
            name: "WEB", version: "2.20260708.00.00", id: "1", userAgent: visionOS.userAgent,
            visitor: true, auth: true, hls: true, cipher: true, itags: [140, 18])
        return [mweb, safari]
    }()

    public static let builtIn = [visionOS] + signedIn

    var label: String { name + (auth == true ? " (signed in)" : "") }

    var context: [String: Any] {
        var c: [String: Any] = ["clientName": name, "clientVersion": version, "userAgent": userAgent,
                                "hl": "en", "gl": "US"]
        c["deviceMake"] = deviceMake
        c["deviceModel"] = deviceModel
        c["osName"] = osName
        c["osVersion"] = osVersion
        return c
    }
}

/// A playable audio URL. googlevideo URLs are bound to this device's IP and
/// expire after ~6 h.
public struct Stream: Sendable {
    public var url: URL
    public var expires: Date
    public var contentLength: Int?
    /// The real length. AVFoundation reads YouTube's fragmented MP4 as twice
    /// as long (measured 2026-10-12: 460.8 s for a 230.4 s song, silence
    /// after the end), so playback must stop here.
    public var duration: TimeInterval?
    /// Loudness relative to YouTube's target in dB (positive: louder).
    public var loudnessDb: Double?
    /// `url` is an HLS playlist.
    public var hls = false
    /// A muxed video (itag 18): only its audio track should play.
    public var hasVideo = false
}

/// Video id → stream URL on the device itself (no yt-dlp): asks the player
/// endpoint as each `ClientProfile` until one returns the AAC stream.
public actor Resolver {
    static let remoteProfiles = URL(string: "https://raw.githubusercontent.com/yaroven/YTMusicPlayer/main/ios/clients.json")!
    static let cacheSize = 16

    let session: URLSession
    let auth: Auth?
    var profiles = ClientProfile.builtIn
    var fetchedProfiles = false
    /// Anonymous and signed-in visitor ids, each kept for an hour.
    var visitors: [Bool: (value: String, at: Date)] = [:]
    var cache: [String: Stream] = [:]
    var order: [String] = []
    /// The player script named by the last signed-in watch page.
    var playerURL: URL?
    let solver: ChallengeSolver

    public init(session: URLSession = .shared, auth: Auth? = nil) {
        self.session = session
        self.auth = auth
        solver = ChallengeSolver(session: session)
    }

    public func stream(_ videoId: String) async throws -> Stream {
        if let hit = cache[videoId], hit.expires.timeIntervalSinceNow > 600 { return hit }
        await loadRemoteProfiles()
        var lastError: Error = YTMError.noStream
        var needsSignIn = false
        for profile in profiles where profile.auth != true {
            do {
                return remember(videoId, try await resolve(videoId, as: profile))
            } catch let error as YTMError {
                if case .signInRequired = error { needsSignIn = true }
                lastError = error
            } catch {
                lastError = error
            }
        }
        guard needsSignIn else { throw lastError }
        guard let auth, auth.signedIn else {
            throw YTMError.signInRequired("Age-restricted: sign in with Google (Library → Google account) to play it")
        }
        for profile in profiles where profile.auth == true {
            do {
                return remember(videoId, try await resolve(videoId, as: profile))
            } catch {
                lastError = error
            }
        }
        throw lastError
    }

    /// One line per client: what the player endpoint answers for the video
    /// and whether the audio actually downloads (for Settings).
    public func diagnose(_ videoId: String) async -> [String] {
        await loadRemoteProfiles()
        var lines: [String] = []
        for profile in profiles where profile.auth != true || auth?.signedIn == true {
            do {
                let stream = try await resolve(videoId, as: profile)
                var request = URLRequest(url: stream.url, timeoutInterval: 10)
                let middle = (stream.contentLength ?? 0) / 2
                if !stream.hls { request.setValue("bytes=\(middle)-\(middle + 65_535)", forHTTPHeaderField: "Range") }
                let status = (try? await session.data(for: request)).flatMap { ($0.1 as? HTTPURLResponse)?.statusCode }
                let ok = status == 206 || (stream.hls && status == 200)
                lines.append("\(ok ? "✓" : "✗") \(profile.label): \(stream.hls ? "HLS" : "AAC") → HTTP \(status.map(String.init) ?? "error")")
            } catch {
                lines.append("✗ \(profile.label): \(error.localizedDescription)")
            }
        }
        if auth?.signedIn != true { lines.append("Not signed in: signed-in clients skipped") }
        return lines
    }

    /// Drop a URL that stopped working (403/410, network change).
    public func forget(_ videoId: String) {
        cache[videoId] = nil
        visitors = [:]
    }

    /// After signing in or out.
    public func reset() {
        cache = [:]
        order = []
        visitors = [:]
    }

    func remember(_ videoId: String, _ stream: Stream) -> Stream {
        cache[videoId] = stream
        order.removeAll { $0 == videoId }
        order.append(videoId)
        if order.count > Self.cacheSize { cache[order.removeFirst()] = nil }
        return stream
    }

    func loadRemoteProfiles() async {
        guard !fetchedProfiles else { return }
        fetchedProfiles = true
        var request = URLRequest(url: Self.remoteProfiles, timeoutInterval: 5)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        guard let (data, response) = try? await session.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let remote = try? JSONDecoder().decode([ClientProfile].self, from: data),
              !remote.isEmpty else { return }
        profiles = remote
    }

    func visitorData(_ videoId: String, profile: ClientProfile, signedIn: Bool) async throws -> String? {
        if let visitor = visitors[signedIn], -visitor.at.timeIntervalSinceNow < 3600 { return visitor.value }
        var request = URLRequest(url: URL(string: "https://www.youtube.com/watch?v=\(videoId)")!)
        request.setValue(profile.userAgent, forHTTPHeaderField: "User-Agent")
        request.setValue("en-US,en", forHTTPHeaderField: "Accept-Language")
        let (data, response) = try await send(request, signedIn: signedIn)
        _ = response
        let page = String(decoding: data, as: UTF8.self)
        if let player = ChallengeSolver.playerURL(inPage: page) { playerURL = player }
        guard let value = visitorDataIn(page) else { return nil }
        visitors[signedIn] = (value, Date())
        return value
    }

    /// Signed-in requests go through the account's own session with its
    /// cookies; anonymous ones never carry them.
    func send(_ request: URLRequest, signedIn: Bool) async throws -> (Data, URLResponse) {
        guard signedIn, let auth else { return try await session.data(for: request) }
        var request = request
        for (name, value) in auth.headers(origin: "https://www.youtube.com") {
            request.setValue(value, forHTTPHeaderField: name)
        }
        let (data, response) = try await auth.session.data(for: request)
        auth.absorb(response)
        return (data, response)
    }

    func resolve(_ videoId: String, as profile: ClientProfile) async throws -> Stream {
        let signedIn = profile.auth == true
        var client = profile.context
        var request = URLRequest(url: URL(string: "https://www.youtube.com/youtubei/v1/player?prettyPrint=false")!)
        if profile.visitor, let visitor = try await visitorData(videoId, profile: profile, signedIn: signedIn) {
            client["visitorData"] = visitor
            request.setValue(visitor, forHTTPHeaderField: "X-Goog-Visitor-Id")
        }
        var body: [String: Any] = ["context": ["client": client], "videoId": videoId,
                                   "contentCheckOk": true, "racyCheckOk": true]
        if profile.cipher == true {
            if playerURL == nil { _ = try await visitorData(videoId, profile: profile, signedIn: signedIn) }
            guard let player = playerURL else { throw YTMError.unplayable("YouTube's player script not found") }
            if let ts = try await solver.signatureTimestamp(player) {
                body["playbackContext"] = ["contentPlaybackContext": ["signatureTimestamp": ts]]
            }
        }
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(profile.userAgent, forHTTPHeaderField: "User-Agent")
        request.setValue(profile.id, forHTTPHeaderField: "X-YouTube-Client-Name")
        request.setValue(profile.version, forHTTPHeaderField: "X-YouTube-Client-Version")
        request.setValue("https://www.youtube.com", forHTTPHeaderField: "Origin")
        let (data, response) = try await send(request, signedIn: signedIn)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw YTMError.http("YouTube player", status) }
        var json = try JSONSerialization.jsonObject(with: data)
        if profile.cipher == true, let player = playerURL, string(json, "playabilityStatus/status") == "OK" {
            json = try await decipherFormats(json, itags: profile.itags ?? [140], player: player, solver: solver)
        }
        var stream = try parsePlayer(json, hls: profile.hls == true, itags: profile.itags ?? [140])
        if stream.hls {
            // The master lists video too; play only its audio playlist.
            let (master, _) = try await session.data(from: stream.url)
            if let audio = audioPlaylist(String(decoding: master, as: UTF8.self), base: stream.url) {
                stream.url = audio
            }
        }
        return stream
    }
}

/// Loudness from the track's measurement when the format lacks one.
func trackLoudness(_ data: Any) -> Double? {
    guard let track = pointer(data, "playerConfig/audioConfig/trackAbsoluteLoudnessLkfs") as? Double,
          let target = pointer(data, "playerConfig/audioConfig/loudnessTargetLkfs") as? Double else {
        return pointer(data, "playerConfig/audioConfig/loudnessDb") as? Double
    }
    return track - target
}

func visitorDataIn(_ html: String) -> String? {
    guard let range = html.range(of: #""VISITOR_DATA":""#) else { return nil }
    let rest = html[range.upperBound...]
    guard let end = rest.firstIndex(of: "\"") else { return nil }
    let value = String(rest[..<end])
    return value.isEmpty ? nil : value
}

/// An HLS master playlist's audio-only rendition (AAC itag 140 if listed).
func audioPlaylist(_ master: String, base: URL) -> URL? {
    let uris = master.split(separator: "\n")
        .filter { $0.hasPrefix("#EXT-X-MEDIA:") && $0.contains("TYPE=AUDIO") }
        .compactMap { line -> String? in
            guard let start = line.range(of: "URI=\"") else { return nil }
            let rest = line[start.upperBound...]
            return rest.firstIndex(of: "\"").map { String(rest[..<$0]) }
        }
    let pick = uris.first { $0.contains("/itag/140/") } ?? uris.first
    return pick.flatMap { URL(string: $0, relativeTo: base)?.absoluteURL }
}

/// The AAC (itag 140) stream of a player response; with `hls`, the HLS
/// manifest when no plain URL is there.
func parsePlayer(_ data: Any, hls: Bool = false, itags: [Int] = [140]) throws -> Stream {
    let status = string(data, "playabilityStatus/status") ?? ""
    guard status == "OK" else {
        let reason = string(data, "playabilityStatus/reason") ?? status
        let message = reason.isEmpty ? "This song can't be played" : reason
        throw status == "LOGIN_REQUIRED" ? YTMError.signInRequired(message) : YTMError.unplayable(message)
    }
    let length = (pointer(data, "videoDetails/lengthSeconds") as? String).flatMap(Double.init)
    let expiresIn = (pointer(data, "streamingData/expiresInSeconds") as? String).flatMap(Double.init)
    let formats = (pointer(data, "streamingData/adaptiveFormats") as? [JSON] ?? [])
        + (pointer(data, "streamingData/formats") as? [JSON] ?? [])
    let wanted = itags.lazy.compactMap { itag in formats.first { $0["itag"] as? Int == itag && $0["url"] != nil } }.first
    guard let aac = wanted
        ?? formats.first(where: { ($0["mimeType"] as? String ?? "").hasPrefix("audio/mp4") && $0["url"] != nil }),
        let url = (aac["url"] as? String).flatMap(URL.init(string:)) else {
        if hls, let manifest = string(data, "streamingData/hlsManifestUrl").flatMap(URL.init(string:)) {
            return Stream(url: manifest, expires: Date().addingTimeInterval(expiresIn ?? 3600),
                          duration: length, loudnessDb: trackLoudness(data), hls: true)
        }
        throw YTMError.noStream
    }
    let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
    let expires = expiresIn
        ?? query.first { $0.name == "expire" }?.value.flatMap(Double.init).map { $0 - Date().timeIntervalSince1970 }
        ?? 3600
    let loudness = aac["loudnessDb"] as? Double ?? trackLoudness(data)
    let duration = (aac["approxDurationMs"] as? String).flatMap(Double.init).map { $0 / 1000 }
        ?? query.first { $0.name == "dur" }?.value.flatMap(Double.init)
        ?? length
    return Stream(url: url, expires: Date().addingTimeInterval(expires),
                  contentLength: (aac["contentLength"] as? String).flatMap(Int.init),
                  duration: duration, loudnessDb: loudness,
                  hasVideo: (aac["mimeType"] as? String ?? "").hasPrefix("video/"))
}
