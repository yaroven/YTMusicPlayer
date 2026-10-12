import CryptoKit
import Foundation

/// A signed-in YouTube session: the youtube.com cookies the sign-in page
/// left behind. Requests carry them plus the `SAPISIDHASH` authorization
/// the web clients send.
public final class Auth: @unchecked Sendable {
    private let lock = NSLock()
    private var cookies: [String: String] = [:]
    /// Called with the updated cookies when YouTube rotates them.
    private let onChange: @Sendable ([String: String]) -> Void

    /// No cookies are stored in or taken from shared storage: they go out
    /// only in the headers built here.
    let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.httpCookieStorage = nil
        config.httpShouldSetCookies = false
        return URLSession(configuration: config)
    }()

    public init(cookies: [String: String] = [:], onChange: @escaping @Sendable ([String: String]) -> Void = { _ in }) {
        self.cookies = cookies
        self.onChange = onChange
    }

    public var signedIn: Bool {
        lock.withLock { Self.isSignedIn(cookies) }
    }

    /// The cookies that make a youtube.com session.
    public static func isSignedIn(_ cookies: [String: String]) -> Bool {
        (cookies["SAPISID"] ?? cookies["__Secure-3PAPISID"]) != nil
            && (cookies["SID"] != nil || cookies["__Secure-3PSID"] != nil)
    }

    public func set(_ cookies: [String: String]) {
        lock.withLock { self.cookies = cookies }
    }

    /// Cookie, Authorization and the headers that go with them.
    func headers(origin: String, now: Date = Date()) -> [String: String] {
        let cookies = lock.withLock { self.cookies }
        guard Self.isSignedIn(cookies) else { return [:] }
        return [
            "Cookie": cookies.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "; "),
            "Authorization": authorization(cookies, origin: origin, now: now),
            "X-Origin": origin,
            "X-Goog-AuthUser": "0",
            "X-Youtube-Bootstrap-Logged-In": "true",
        ]
    }

    /// Keep cookies YouTube rotates (`__Secure-*PSIDTS` etc.).
    func absorb(_ response: URLResponse) {
        guard let http = response as? HTTPURLResponse, let url = http.url,
              let fields = http.allHeaderFields as? [String: String] else { return }
        let fresh = HTTPCookie.cookies(withResponseHeaderFields: fields, for: url)
            .filter { $0.domain.hasSuffix("youtube.com") }
        guard !fresh.isEmpty else { return }
        let updated: [String: String]? = lock.withLock {
            guard Self.isSignedIn(cookies) else { return nil }
            var changed = false
            for cookie in fresh where cookies[cookie.name] != nil && cookies[cookie.name] != cookie.value {
                cookies[cookie.name] = cookie.value
                changed = true
            }
            return changed ? cookies : nil
        }
        if let updated { onChange(updated) }
    }
}

/// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`, plus the 1P / 3P
/// variants, as youtube.com sends it.
func authorization(_ cookies: [String: String], origin: String, now: Date) -> String {
    let ts = String(Int(now.timeIntervalSince1970))
    func hash(_ value: String) -> String {
        Insecure.SHA1.hash(data: Data("\(ts) \(value) \(origin)".utf8)).map { String(format: "%02x", $0) }.joined()
    }
    let schemes: [(String, String?)] = [
        ("SAPISIDHASH", cookies["SAPISID"] ?? cookies["__Secure-3PAPISID"]),
        ("SAPISID1PHASH", cookies["__Secure-1PAPISID"]),
        ("SAPISID3PHASH", cookies["__Secure-3PAPISID"]),
    ]
    return schemes.compactMap { scheme, value in value.map { "\(scheme) \(ts)_\(hash($0))" } }
        .joined(separator: " ")
}
