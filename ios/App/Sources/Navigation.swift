import Foundation
import YTMKit

/// The app's one copy of each YTMKit service.
enum Services {
    static let catalog = Catalog()
    static let resolver = Resolver()
    static let lyrics = LyricsFinder(catalog: catalog)
}

enum AppTab: Hashable {
    case home, search, library
}

enum Route: Hashable {
    /// Album, artist or playlist page by browse id.
    case page(String)
}

/// Each tab's navigation stack, so "Go to artist" works from anywhere.
@MainActor @Observable
final class Router {
    var tab: AppTab = .home
    var home: [Route] = []
    var search: [Route] = []
    var library: [Route] = []

    func open(_ route: Route) {
        switch tab {
        case .home: home.append(route)
        case .search: search.append(route)
        case .library: library.append(route)
        }
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
