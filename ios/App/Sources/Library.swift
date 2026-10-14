import Foundation
import YTMKit

struct LocalPlaylist: Codable, Identifiable, Hashable {
    var id = UUID()
    var name: String
    var tracks: [Track] = []
    /// YouTube's id (`PL…`) when this playlist is the account's own.
    var remoteId: String?
    /// Per track, the id a removal needs (same order as `tracks`).
    var setVideoIds: [String] = []
}

/// A saved album, artist or playlist page.
struct SavedPage: Codable, Identifiable, Hashable {
    var id: String
    var kind: ItemKind
    var title: String
    var subtitle: String
    var thumbnail: URL?
    /// Came from the account (replaced by every sync) rather than saved here.
    var synced = false
}

enum LocalList: Hashable {
    case liked, history
    case playlist(UUID)
}

/// The library: liked songs, playlists and saved pages of the signed-in
/// account (synced from YouTube Music and written back to it), plus history
/// and anything saved while signed out. One small JSON file in Application
/// Support.
@MainActor @Observable
final class Library {
    static let historySize = 200

    private(set) var liked: [Track] = []
    private(set) var history: [Track] = []
    private(set) var playlists: [LocalPlaylist] = []
    private(set) var saved: [SavedPage] = []
    /// A sync is running.
    private(set) var syncing = false
    /// Last failure of a sync or of a write back to YouTube.
    var error: String?

    @ObservationIgnored private let file: URL
    @ObservationIgnored private let catalog: Catalog
    @ObservationIgnored private var signedIn: Bool { catalog.signedIn }

    private struct Stored: Codable {
        var liked: [Track]?
        var history: [Track]?
        var playlists: [LocalPlaylist]?
        var saved: [SavedPage]?
    }

    init(file: URL? = nil, catalog: Catalog = Services.catalog) {
        self.catalog = catalog
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

    // MARK: The account

    /// True before the first sync of a signed-in account.
    var needsFirstSync: Bool {
        signedIn && liked.isEmpty && !playlists.contains { $0.remoteId != nil }
    }

    /// Replaces the account's part of the library — liked songs, its
    /// playlists, saved albums and followed artists — with what YouTube
    /// Music has. Anything made on the phone while signed out is kept.
    func sync() async {
        guard signedIn, !syncing else { return }
        syncing = true
        defer { syncing = false }
        do {
            let likedSongs = try await catalog.likedSongs()
            var remote: [LocalPlaylist] = []
            for item in try await catalog.libraryPlaylists() {
                let entries = try await catalog.playlistEntries(item.browseId)
                remote.append(LocalPlaylist(name: item.title,
                                            tracks: entries.map(\.track),
                                            remoteId: withoutVL(item.browseId),
                                            setVideoIds: entries.map(\.setVideoId)))
            }
            let pages = try await catalog.libraryAlbums() + catalog.libraryArtists()
            liked = likedSongs
            playlists = remote + playlists.filter { $0.remoteId == nil }
            let fromAccount = pages.map {
                SavedPage(id: $0.browseId, kind: $0.kind, title: $0.title, subtitle: $0.subtitle,
                          thumbnail: $0.thumbnail, synced: true)
            }
            let ids = Set(fromAccount.map(\.id))
            saved = fromAccount + saved.filter { !$0.synced && !ids.contains($0.id) }
            save()
            error = nil
        } catch {
            self.error = "Sync failed: \(error.localizedDescription)"
        }
    }

    /// Forgets what the account contributed (after signing out).
    func forgetAccount() {
        liked = []
        playlists = playlists.filter { $0.remoteId == nil }
        saved = saved.filter { !$0.synced }
        save()
    }

    /// Sends a library change to YouTube in the background; signed out, the
    /// change only lives on the phone. `undo` puts the local state back when
    /// YouTube refuses it.
    private func write(_ what: String,
                       _ body: @escaping @MainActor () async throws -> Void,
                       undo: (@MainActor () -> Void)? = nil) {
        guard signedIn else { return }
        Task { @MainActor in
            do {
                try await body()
            } catch {
                self.error = "\(what) failed: \(error.localizedDescription)"
                undo?()
            }
        }
    }

    /// Browse ids carry a `VL` prefix; playlist ids travel without it.
    private func withoutVL(_ id: String) -> String {
        id.hasPrefix("VL") ? String(id.dropFirst(2)) : id
    }

    // MARK: Songs

    func isLiked(_ track: Track) -> Bool {
        liked.contains { $0.videoId == track.videoId }
    }

    /// Likes or unlikes a song — in the account too, when signed in.
    func toggleLike(_ track: Track) {
        let liking = !isLiked(track)
        if liking {
            liked.insert(track, at: 0)
        } else {
            liked.removeAll { $0.videoId == track.videoId }
        }
        save()
        write("\(liking ? "Liking" : "Unliking") \(track.title)") { [catalog] in
            try await catalog.rate(track.videoId, liking ? .like : .none)
        } undo: { [weak self] in
            guard let self else { return }
            if liking {
                liked.removeAll { $0.videoId == track.videoId }
            } else {
                liked.insert(track, at: 0)
            }
            save()
        }
    }

    /// Dislikes a song (YouTube learns from it and drops the like).
    func dislike(_ track: Track) {
        liked.removeAll { $0.videoId == track.videoId }
        save()
        write("Disliking \(track.title)") { [catalog] in
            try await catalog.rate(track.videoId, .dislike)
        }
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
        case .liked:
            let removed = offsets.map { liked[$0] }
            liked.remove(atOffsets: offsets)
            save()
            for track in removed {
                write("Unliking \(track.title)") { [catalog] in
                    try await catalog.rate(track.videoId, .none)
                }
            }
        case .history:
            history.remove(atOffsets: offsets)
            save()
        case let .playlist(id):
            guard let i = playlists.firstIndex(where: { $0.id == id }) else { return }
            let remoteId = playlists[i].remoteId
            let entries = offsets.map {
                (track: playlists[i].tracks[$0],
                 setVideoId: playlists[i].setVideoIds.indices.contains($0) ? playlists[i].setVideoIds[$0] : "")
            }
            playlists[i].tracks.remove(atOffsets: offsets)
            playlists[i].setVideoIds = offsets.reversed().reduce(playlists[i].setVideoIds) { ids, at in
                var ids = ids
                if ids.indices.contains(at) { ids.remove(at: at) }
                return ids
            }
            save()
            guard let remoteId else { return }
            for entry in entries where !entry.setVideoId.isEmpty {
                write("Removing \(entry.track.title)") { [catalog] in
                    try await catalog.removeFromPlaylist(remoteId, entry.track.videoId,
                                                         setVideoId: entry.setVideoId)
                }
            }
        }
    }

    func move(from source: IndexSet, to destination: Int, in list: LocalList) {
        guard case let .playlist(id) = list, let i = playlists.firstIndex(where: { $0.id == id }) else { return }
        playlists[i].tracks.move(fromOffsets: source, toOffset: destination)
        save()
    }

    @discardableResult
    func createPlaylist(_ name: String, with track: Track? = nil) -> UUID {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let title = trimmed.isEmpty ? "Playlist \(playlists.count + 1)" : trimmed
        let playlist = LocalPlaylist(name: title, tracks: track.map { [$0] } ?? [],
                                     setVideoIds: track.map { _ in [""] } ?? [])
        playlists.append(playlist)
        save()
        let id = playlist.id
        write("Creating \(title)") { [self] in
            let remoteId = try await catalog.createPlaylist(title)
            var setVideoId: String?
            if let track {
                setVideoId = try await catalog.addToPlaylist(remoteId, track.videoId)
            }
            guard let i = playlists.firstIndex(where: { $0.id == id }) else { return }
            playlists[i].remoteId = remoteId
            if let setVideoId { playlists[i].setVideoIds = [setVideoId] }
            save()
        } undo: { [self] in
            playlists.removeAll { $0.id == id }
            save()
        }
        return id
    }

    /// Deletes playlists here and, for the account's own, on YouTube too.
    func deletePlaylists(at offsets: IndexSet) {
        let removed = offsets.map { playlists[$0] }
        playlists.remove(atOffsets: offsets)
        save()
        for playlist in removed {
            guard let remoteId = playlist.remoteId else { continue }
            write("Deleting \(playlist.name)") { [catalog] in
                try await catalog.deletePlaylist(remoteId)
            }
        }
    }

    func add(_ track: Track, to playlist: UUID) {
        guard let i = playlists.firstIndex(where: { $0.id == playlist }),
              !playlists[i].tracks.contains(where: { $0.videoId == track.videoId }) else { return }
        playlists[i].tracks.append(track)
        playlists[i].setVideoIds.append("")
        save()
        guard let remoteId = playlists[i].remoteId else { return }
        let id = playlist
        write("Adding \(track.title)") { [self] in
            let setVideoId = try await catalog.addToPlaylist(remoteId, track.videoId)
            guard let i = playlists.firstIndex(where: { $0.id == id }),
                  let at = playlists[i].tracks.lastIndex(where: { $0.videoId == track.videoId })
            else { return }
            while playlists[i].setVideoIds.count <= at { playlists[i].setVideoIds.append("") }
            playlists[i].setVideoIds[at] = setVideoId
            save()
        } undo: { [self] in
            guard let i = playlists.firstIndex(where: { $0.id == id }),
                  let at = playlists[i].tracks.lastIndex(where: { $0.videoId == track.videoId })
            else { return }
            playlists[i].tracks.remove(at: at)
            if playlists[i].setVideoIds.indices.contains(at) { playlists[i].setVideoIds.remove(at: at) }
            save()
        }
    }

    // MARK: Saved pages

    func isSaved(_ id: String) -> Bool {
        saved.contains { $0.id == id }
    }

    /// Saves an album or playlist page, or follows an artist — subscribing
    /// on YouTube when signed in.
    func toggleSaved(_ page: Page) {
        let saving = !isSaved(page.id)
        if saving {
            saved.insert(SavedPage(id: page.id, kind: page.kind, title: page.title,
                                   subtitle: page.subtitle, thumbnail: page.thumbnail,
                                   synced: page.kind == .artist && signedIn), at: 0)
        } else {
            saved.removeAll { $0.id == page.id }
        }
        save()
        // An artist page's id is its channel id, which is what a
        // subscription needs.
        guard page.kind == .artist else { return }
        let channel = page.id
        write("\(saving ? "Following" : "Unfollowing") \(page.title)") { [catalog] in
            try await catalog.subscribe(channel, on: saving)
        }
    }

    func deleteSaved(at offsets: IndexSet) {
        let removed = offsets.map { saved[$0] }
        saved.remove(atOffsets: offsets)
        save()
        for page in removed where page.kind == .artist {
            write("Unfollowing \(page.title)") { [catalog] in
                try await catalog.subscribe(page.id, on: false)
            }
        }
    }
}
