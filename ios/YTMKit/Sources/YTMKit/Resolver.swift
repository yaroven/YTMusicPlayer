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

    /// Measured 2026-10-12: itag 140 URL, every range 206, AppleCoreMedia UA
    /// accepted. IOS URLs 403 past the first chunk (PO token); ANDROID_VR
    /// and TVHTML5 hit "confirm you're not a bot".
    public static let visionOS = ClientProfile(
        name: "VISIONOS", version: "1.02", id: "101",
        userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
        deviceMake: "Apple", deviceModel: "RealityDevice17,1",
        osName: "visionOS", osVersion: "26.5.23O471", visitor: true)

    public static let builtIn = [visionOS]

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
}

/// Video id → stream URL on the device itself (no yt-dlp): asks the player
/// endpoint as each `ClientProfile` until one returns the AAC stream.
public actor Resolver {
    static let remoteProfiles = URL(string: "https://raw.githubusercontent.com/yaroven/YTMusicPlayer/main/ios/clients.json")!
    static let cacheSize = 16

    let session: URLSession
    var profiles = ClientProfile.builtIn
    var fetchedProfiles = false
    var visitor: (value: String, at: Date)?
    var cache: [String: Stream] = [:]
    var order: [String] = []

    public init(session: URLSession = .shared) {
        self.session = session
    }

    public func stream(_ videoId: String) async throws -> Stream {
        if let hit = cache[videoId], hit.expires.timeIntervalSinceNow > 600 { return hit }
        await loadRemoteProfiles()
        var lastError: Error = YTMError.noStream
        for profile in profiles {
            do {
                let stream = try await resolve(videoId, as: profile)
                remember(videoId, stream)
                return stream
            } catch let error as YTMError {
                // Age restriction etc. won't change with another client.
                if case .unplayable = error, profile == profiles.last { throw error }
                lastError = error
            } catch {
                lastError = error
            }
        }
        throw lastError
    }

    /// Drop a URL that stopped working (403/410, network change).
    public func forget(_ videoId: String) {
        cache[videoId] = nil
        visitor = nil
    }

    func remember(_ videoId: String, _ stream: Stream) {
        cache[videoId] = stream
        order.removeAll { $0 == videoId }
        order.append(videoId)
        if order.count > Self.cacheSize { cache[order.removeFirst()] = nil }
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

    /// youtube.com's visitor id, kept for an hour.
    func visitorData(_ videoId: String, userAgent: String) async throws -> String? {
        if let visitor, -visitor.at.timeIntervalSinceNow < 3600 { return visitor.value }
        var request = URLRequest(url: URL(string: "https://www.youtube.com/watch?v=\(videoId)")!)
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        request.setValue("en-US,en", forHTTPHeaderField: "Accept-Language")
        let (data, _) = try await session.data(for: request)
        guard let value = visitorDataIn(String(decoding: data, as: UTF8.self)) else { return nil }
        visitor = (value, Date())
        return value
    }

    func resolve(_ videoId: String, as profile: ClientProfile) async throws -> Stream {
        var client = profile.context
        var request = URLRequest(url: URL(string: "https://www.youtube.com/youtubei/v1/player?prettyPrint=false")!)
        if profile.visitor, let visitor = try await visitorData(videoId, userAgent: profile.userAgent) {
            client["visitorData"] = visitor
            request.setValue(visitor, forHTTPHeaderField: "X-Goog-Visitor-Id")
        }
        let body: [String: Any] = ["context": ["client": client], "videoId": videoId,
                                   "contentCheckOk": true, "racyCheckOk": true]
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(profile.userAgent, forHTTPHeaderField: "User-Agent")
        request.setValue(profile.id, forHTTPHeaderField: "X-YouTube-Client-Name")
        request.setValue(profile.version, forHTTPHeaderField: "X-YouTube-Client-Version")
        request.setValue("https://www.youtube.com", forHTTPHeaderField: "Origin")
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw YTMError.http("YouTube player", status) }
        return try parsePlayer(try JSONSerialization.jsonObject(with: data))
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

/// The AAC (itag 140) stream of a player response.
func parsePlayer(_ data: Any) throws -> Stream {
    let status = string(data, "playabilityStatus/status") ?? ""
    guard status == "OK" else {
        let reason = string(data, "playabilityStatus/reason") ?? status
        throw YTMError.unplayable(reason.isEmpty ? "This song can't be played" : reason)
    }
    let formats = pointer(data, "streamingData/adaptiveFormats") as? [JSON] ?? []
    guard let aac = formats.first(where: { $0["itag"] as? Int == 140 && $0["url"] != nil })
        ?? formats.first(where: { ($0["mimeType"] as? String ?? "").hasPrefix("audio/mp4") && $0["url"] != nil }),
        let url = (aac["url"] as? String).flatMap(URL.init(string:)) else { throw YTMError.noStream }
    let expiresIn = (pointer(data, "streamingData/expiresInSeconds") as? String).flatMap(Double.init)
        ?? URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?
        .first { $0.name == "expire" }?.value.flatMap(Double.init).map { $0 - Date().timeIntervalSince1970 }
        ?? 3600
    let loudness = aac["loudnessDb"] as? Double ?? trackLoudness(data)
    let duration = (aac["approxDurationMs"] as? String).flatMap(Double.init).map { $0 / 1000 }
        ?? URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?
        .first { $0.name == "dur" }?.value.flatMap(Double.init)
    return Stream(url: url, expires: Date().addingTimeInterval(expiresIn),
                  contentLength: (aac["contentLength"] as? String).flatMap(Int.init),
                  duration: duration, loudnessDb: loudness)
}
