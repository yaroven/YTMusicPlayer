import Foundation
import YTMKit

struct LocalPlaylist: Codable, Identifiable, Hashable {
    var id = UUID()
    var name: String
    var tracks: [Track] = []
}

/// A saved album, artist or playlist page.
struct SavedPage: Codable, Identifiable, Hashable {
    var id: String
    var kind: ItemKind
    var title: String
    var subtitle: String
    var thumbnail: URL?
}

enum LocalList: Hashable {
    case liked, history
    case playlist(UUID)
}

/// The on-device library: liked songs, history, playlists, saved pages.
/// One small JSON file in Application Support.
@MainActor @Observable
final class Library {
    static let historySize = 200

    private(set) var liked: [Track] = []
    private(set) var history: [Track] = []
    private(set) var playlists: [LocalPlaylist] = []
    private(set) var saved: [SavedPage] = []

    @ObservationIgnored private let file: URL

    private struct Stored: Codable {
        var liked: [Track]?
        var history: [Track]?
        var playlists: [LocalPlaylist]?
        var saved: [SavedPage]?
    }

    init(file: URL? = nil) {
        self.file = file ?? Self.directory.appendingPathComponent("library.json")
        if let data = try? Data(contentsOf: self.file),
           let stored = try? JSONDecoder().decode(Stored.self, from: data) {
            liked = stored.liked ?? []
            history = stored.history ?? []
            playlists = stored.playlists ?? []
            saved = stored.saved ?? []
        }
    }

    static var directory: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func save() {
        let stored = Stored(liked: liked, history: history, playlists: playlists, saved: saved)
        guard let data = try? JSONEncoder().encode(stored) else { return }
        try? data.write(to: file, options: .atomic)
    }

    // MARK: Songs

    func isLiked(_ track: Track) -> Bool {
        liked.contains { $0.videoId == track.videoId }
    }

    func toggleLike(_ track: Track) {
        if isLiked(track) {
            liked.removeAll { $0.videoId == track.videoId }
        } else {
            liked.insert(track, at: 0)
        }
        save()
    }

    func played(_ track: Track) {
        history.removeAll { $0.videoId == track.videoId }
        history.insert(track, at: 0)
        if history.count > Self.historySize { history.removeLast(history.count - Self.historySize) }
        save()
    }

    func clearHistory() {
        history = []
        save()
    }

    // MARK: Lists

    func tracks(_ list: LocalList) -> [Track] {
        switch list {
        case .liked: liked
        case .history: history
        case let .playlist(id): playlists.first { $0.id == id }?.tracks ?? []
        }
    }

    func title(_ list: LocalList) -> String {
        switch list {
        case .liked: "Liked songs"
        case .history: "History"
        case let .playlist(id): playlists.first { $0.id == id }?.name ?? "Playlist"
        }
    }

    func remove(at offsets: IndexSet, from list: LocalList) {
        switch list {
        case .liked: liked.remove(atOffsets: offsets)
        case .history: history.remove(atOffsets: offsets)
        case let .playlist(id):
            guard let i = playlists.firstIndex(where: { $0.id == id }) else { return }
            playlists[i].tracks.remove(atOffsets: offsets)
        }
        save()
    }

    func move(from source: IndexSet, to destination: Int, in list: LocalList) {
        guard case let .playlist(id) = list, let i = playlists.firstIndex(where: { $0.id == id }) else { return }
        playlists[i].tracks.move(fromOffsets: source, toOffset: destination)
        save()
    }

    @discardableResult
    func createPlaylist(_ name: String, with track: Track? = nil) -> UUID {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let playlist = LocalPlaylist(name: trimmed.isEmpty ? "Playlist \(playlists.count + 1)" : trimmed,
                                     tracks: track.map { [$0] } ?? [])
        playlists.append(playlist)
        save()
        return playlist.id
    }

    func deletePlaylists(at offsets: IndexSet) {
        playlists.remove(atOffsets: offsets)
        save()
    }

    func add(_ track: Track, to playlist: UUID) {
        guard let i = playlists.firstIndex(where: { $0.id == playlist }),
              !playlists[i].tracks.contains(where: { $0.videoId == track.videoId }) else { return }
        playlists[i].tracks.append(track)
        save()
    }

    // MARK: Saved pages

    func isSaved(_ id: String) -> Bool {
        saved.contains { $0.id == id }
    }

    func toggleSaved(_ page: Page) {
        if isSaved(page.id) {
            saved.removeAll { $0.id == page.id }
        } else {
            saved.insert(SavedPage(id: page.id, kind: page.kind, title: page.title,
                                   subtitle: page.subtitle, thumbnail: page.thumbnail), at: 0)
        }
        save()
    }

    func deleteSaved(at offsets: IndexSet) {
        saved.remove(atOffsets: offsets)
        save()
    }
}
