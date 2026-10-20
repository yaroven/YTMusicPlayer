import Foundation
import YTMKit

/// The app's one copy of each YTMKit service.
enum Services {
    /// The Google account's youtube.com cookies, kept in the Keychain
    /// (rotated cookies are saved back as YouTube sends them).
    static let auth = Auth(cookies: Keychain.cookies() ?? [:], onChange: { Keychain.save($0) })
    /// Shelves and titles in the app's language (iOS's per-app setting).
    static let catalog = Catalog(auth: auth,
                                 language: Bundle.main.preferredLocalizations.first == "uk" ? "uk" : "en")
    static let resolver = Resolver(auth: auth)
    static let lyrics = LyricsFinder(catalog: catalog)
}

enum AppTab: Hashable {
    case home, explore, library
}

enum Route: Hashable {
    /// Album, artist or playlist page by browse id.
    case page(String)
    /// The songs on the phone (offline, from an error screen).
    case downloads
    /// Search, opened from the magnifier in a screen's header.
    case search
}

/// Each tab's navigation stack, so "Go to artist" works from anywhere.
@MainActor @Observable
final class Router {
    var tab: AppTab = .home
    var home: [Route] = []
    var explore: [Route] = []
    var library: [Route] = []

    func open(_ route: Route) {
        switch tab {
        case .home: home.append(route)
        case .explore: explore.append(route)
        case .library: library.append(route)
        }
    }

    /// Offline: straight to what still plays.
    func openDownloads() {
        tab = .library
        library.append(.downloads)
    }

    /// The song's artist page, as linked under the song, else the first
    /// artist search hit for the name.
    func openArtist(of track: Track) {
        Task {
            if let id = try? await Services.catalog.artistId(track.videoId) {
                open(.page(id))
                return
            }
            let name = track.artist.components(separatedBy: ",").first ?? track.artist
            if let hit = try? await Services.catalog.search(name, kind: .artists).first {
                open(.page(hit.browseId))
            }
        }
    }
}

func formatTime(_ seconds: Double) -> String {
    guard seconds.isFinite, seconds >= 0 else { return "0:00" }
    let s = Int(seconds)
    return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60)
        : String(format: "%d:%02d", s / 60, s % 60)
}
