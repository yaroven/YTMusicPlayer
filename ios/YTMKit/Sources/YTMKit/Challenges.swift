import Foundation
#if canImport(JavaScriptCore)
import JavaScriptCore
#endif

/// The player script a web client's formats belong to: its URL and the
/// `signatureTimestamp` the player request must send.
struct PlayerScript: Sendable {
    var url: URL
    var signatureTimestamp: Int?
}

/// YouTube's signature ("s") and throttling ("n") challenges, solved with
/// yt-dlp's own solver (yt-dlp-ejs 0.8.0, Unlicense; bundles meriyah and
/// astring) in JavaScriptCore. Needed only for the signed-in web clients
/// that serve age-restricted songs; the player script (~2.5 MB) is
/// downloaded once per player version and only its preprocessed form kept.
actor ChallengeSolver {
    let session: URLSession
    /// Player URL → preprocessed player (what the solver returns to reuse).
    private var preprocessed: [URL: String] = [:]
    private var timestamps: [URL: Int] = [:]

    init(session: URLSession) {
        self.session = session
    }

    /// `"jsUrl":"/s/player/<id>/player_es6_tcc.vflset/en_US/base.js"` in a
    /// watch page, as the "main" variant yt-dlp's solver understands.
    nonisolated static func playerURL(inPage html: String) -> URL? {
        guard let range = html.range(of: #""jsUrl":""#) else { return nil }
        let rest = html[range.upperBound...]
        guard let end = rest.firstIndex(of: "\"") else { return nil }
        var path = String(rest[..<end])
        if let player = path.range(of: #"/s/player/[0-9a-zA-Z_-]+/"#, options: .regularExpression) {
            path = path[..<player.upperBound] + "player_ias.vflset/en_US/base.js"
        }
        return URL(string: path, relativeTo: URL(string: "https://www.youtube.com")!)?.absoluteURL
    }

    /// The player's `signatureTimestamp` (downloads the player once).
    func signatureTimestamp(_ url: URL) async throws -> Int? {
        if let ts = timestamps[url] { return ts }
        let code = try await download(url)
        let ts = Self.signatureTimestamp(in: code)
        timestamps[url] = ts
        try await preprocess(url, code: code)
        return ts
    }

    nonisolated static func signatureTimestamp(in code: String) -> Int? {
        for pattern in [#"signatureTimestamp[:=](\d+)"#, #"sts:(\d+)"#] {
            if let r = code.range(of: pattern, options: .regularExpression) {
                let digits = code[r].drop { !$0.isNumber }
                if let ts = Int(digits) { return ts }
            }
        }
        return nil
    }

    /// Solutions for each challenge, by challenge.
    func solve(player url: URL, sig: [String], n: [String]) async throws -> (sig: [String: String], n: [String: String]) {
        if sig.isEmpty && n.isEmpty { return ([:], [:]) }
        var requests: [[String: Any]] = []
        if !sig.isEmpty { requests.append(["type": "sig", "challenges": sig]) }
        if !n.isEmpty { requests.append(["type": "n", "challenges": n]) }
        var input: [String: Any] = ["requests": requests]
        if let cached = preprocessed[url] {
            input["type"] = "preprocessed"
            input["preprocessed_player"] = cached
        } else {
            input["type"] = "player"
            input["player"] = try await download(url)
            input["output_preprocessed"] = true
        }
        let output = try Self.run(input)
        if let player = output["preprocessed_player"] as? String { preprocessed[url] = player }
        let responses = output["responses"] as? [[String: Any]] ?? []
        var solved = (sig: [String: String](), n: [String: String]())
        for (request, response) in zip(requests, responses) {
            guard response["type"] as? String == "result", let data = response["data"] as? [String: String] else {
                throw YTMError.unplayable("YouTube's player changed (\(response["error"] ?? "no result"))")
            }
            if request["type"] as? String == "sig" { solved.sig.merge(data) { $1 } } else { solved.n.merge(data) { $1 } }
        }
        return solved
    }

    private func preprocess(_ url: URL, code: String) async throws {
        guard preprocessed[url] == nil else { return }
        let output = try Self.run(["type": "player", "player": code, "requests": [], "output_preprocessed": true])
        if let player = output["preprocessed_player"] as? String { preprocessed[url] = player }
    }

    private func download(_ url: URL) async throws -> String {
        let (data, response) = try await session.data(from: url)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw YTMError.http("YouTube player script", status) }
        return String(decoding: data, as: UTF8.self)
    }

    /// One fresh JSContext per call: nothing of the 2.5 MB player stays.
    private static func run(_ input: [String: Any]) throws -> [String: Any] {
        #if canImport(JavaScriptCore)
        guard let lib = script("yt.solver.lib.min"), let core = script("yt.solver.core.min"),
              let context = JSContext() else { throw YTMError.unplayable("JavaScript solver unavailable") }
        var failure: String?
        context.exceptionHandler = { _, exception in failure = exception?.toString() }
        context.evaluateScript(lib)
        context.evaluateScript("Object.assign(globalThis, lib);")
        context.evaluateScript(core)
        let json = String(decoding: try JSONSerialization.data(withJSONObject: input), as: UTF8.self)
        context.setObject(json, forKeyedSubscript: "ytmInput" as NSString)
        let result = context.evaluateScript("JSON.stringify(jsc(JSON.parse(ytmInput)))")?.toString()
        if let failure { throw YTMError.unplayable("JavaScript solver: \(failure)") }
        guard let result, let output = try JSONSerialization.jsonObject(with: Data(result.utf8)) as? [String: Any] else {
            throw YTMError.unplayable("JavaScript solver returned nothing")
        }
        if output["type"] as? String == "error" {
            throw YTMError.unplayable("JavaScript solver: \(output["error"] ?? "error")")
        }
        return output
        #else
        throw YTMError.unplayable("JavaScript solver unavailable")
        #endif
    }

    private static func script(_ name: String) -> String? {
        Bundle.module.url(forResource: name, withExtension: "js")
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
    }
}

/// Plain URLs for the wanted formats of a web client's player response:
/// `signatureCipher` (s, sp, url) gets its signature, every URL its "n"
/// solved. Formats left ciphered are dropped.
func decipherFormats(_ data: Any, itags: [Int], player: URL, solver: ChallengeSolver) async throws -> Any {
    guard var root = data as? JSON, var streaming = root["streamingData"] as? JSON else { return data }
    struct Pending { var key: String; var index: Int; var url: URLComponents; var s: String?; var sp: String }
    var pending: [Pending] = []
    for key in ["adaptiveFormats", "formats"] {
        for (i, format) in (streaming[key] as? [JSON] ?? []).enumerated() {
            guard let itag = format["itag"] as? Int, itags.contains(itag) else { continue }
            if let plain = format["url"] as? String, let url = URLComponents(string: plain) {
                pending.append(Pending(key: key, index: i, url: url, s: nil, sp: "sig"))
            } else if let cipher = format["signatureCipher"] as? String {
                let query = URLComponents(string: "?" + cipher)?.queryItems ?? []
                func value(_ name: String) -> String? { query.first { $0.name == name }?.value }
                guard let raw = value("url"), let url = URLComponents(string: raw) else { continue }
                pending.append(Pending(key: key, index: i, url: url, s: value("s"), sp: value("sp") ?? "signature"))
            }
        }
    }
    let n = pending.compactMap { $0.url.queryItems?.first { $0.name == "n" }?.value }
    let solved = try await solver.solve(player: player, sig: pending.compactMap(\.s), n: Array(Set(n)))
    for item in pending {
        var url = item.url
        var query = url.queryItems ?? []
        if let s = item.s {
            guard let signature = solved.sig[s] else { continue }
            query.append(URLQueryItem(name: item.sp, value: signature))
        }
        if let i = query.firstIndex(where: { $0.name == "n" }), let value = query[i].value, let fixed = solved.n[value] {
            query[i].value = fixed
        }
        url.queryItems = query
        guard var formats = streaming[item.key] as? [JSON], let plain = url.url?.absoluteString else { continue }
        formats[item.index]["url"] = plain
        formats[item.index]["signatureCipher"] = nil
        streaming[item.key] = formats
    }
    root["streamingData"] = streaming
    return root
}
