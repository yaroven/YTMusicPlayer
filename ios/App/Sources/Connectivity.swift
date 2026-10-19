import Foundation
import Network
import YTMKit

/// Whether the phone has a usable internet connection (NWPathMonitor), for
/// the banner, screens that load again when it's back, and the player
/// waiting instead of skipping.
@MainActor @Observable
final class Connectivity {
    static let shared = Connectivity()

    private(set) var online = true
    /// Bumped each time the connection comes back; views retry on it.
    private(set) var cameBack = 0

    @ObservationIgnored private let monitor = NWPathMonitor()
    @ObservationIgnored private var reconnected: [@MainActor () -> Void] = []

    private init() {
        monitor.pathUpdateHandler = { [weak self] path in
            let up = path.status == .satisfied
            Task { @MainActor in self?.set(up) }
        }
        monitor.start(queue: DispatchQueue(label: "connectivity"))
    }

    /// Runs `action` whenever the connection comes back.
    func onReconnect(_ action: @escaping @MainActor () -> Void) {
        reconnected.append(action)
    }

    private func set(_ up: Bool) {
        let back = up && !online
        online = up
        if back {
            cameBack += 1
            reconnected.forEach { $0() }
        }
    }
}

/// What went wrong, in the user's terms.
enum Problem {
    case offline, slow, server, signIn, other
}

extension Error {
    @MainActor var problem: Problem {
        if !Connectivity.shared.online { return .offline }
        return Self.problem(of: self as NSError)
    }

    private static func problem(of error: NSError) -> Problem {
        if let ytm = (error as Error) as? YTMError {
            switch ytm {
            case let .http(_, status) where status >= 500: return .server
            case .signInRequired: return .signIn
            default: return .other
            }
        }
        if error.domain == NSURLErrorDomain {
            switch URLError.Code(rawValue: error.code) {
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost,
                 .dnsLookupFailed, .dataNotAllowed, .internationalRoamingOff, .secureConnectionFailed:
                return .offline
            case .timedOut:
                return .slow
            default:
                break
            }
        }
        // AVFoundation wraps the URL error.
        if let underlying = error.userInfo[NSUnderlyingErrorKey] as? NSError {
            return problem(of: underlying)
        }
        return .other
    }

    /// One line: "No internet connection", the reason YouTube gave…
    @MainActor var friendly: String {
        switch problem {
        case .offline: String(localized: "No internet connection")
        case .slow: String(localized: "The connection is too slow")
        case .server: String(localized: "YouTube Music isn't answering")
        case .signIn, .other: localizedDescription
        }
    }

    @MainActor var isNetwork: Bool {
        problem == .offline || problem == .slow
    }
}

/// One-line messages at the bottom of the screen.
@MainActor @Observable
final class Notice {
    static let shared = Notice()
    private(set) var text: String?
    @ObservationIgnored private var hide: Task<Void, Never>?

    func show(_ message: String) {
        text = message
        hide?.cancel()
        hide = Task {
            try? await Task.sleep(for: .seconds(3))
            if !Task.isCancelled { text = nil }
        }
    }
}
