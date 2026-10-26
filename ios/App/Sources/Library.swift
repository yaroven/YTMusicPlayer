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

/// A song saved on the phone for offline play.
struct Downloaded: Codable, Hashable {
    var track: Track
    /// File name inside `Library.downloadsDirectory`.
    var file: String
    var bytes: Int
    /// The real length (AVFoundation reads YouTube's fMP4 as twice as long).
    var duration: TimeInterval?
    var loudnessDb: Double?
}

enum LocalList: Hashable {
    case liked, history, downloads
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
    /// Songs on the phone, newest first.
    private(set) var downloads: [Downloaded] = []
    /// Songs being downloaded now.
    private(set) var downloading: Set<String> = []
    /// How far a batch got: done of total ("Downloading 3 of 12").
    private(set) var batchDone = 0
    private(set) var batchTotal = 0
    /// How often each song was played (by video id); outlives the capped history.
    private(set) var plays: [String: Int] = [:]
    @ObservationIgnored private var queue: [Track] = []
    @ObservationIgnored private var worker: Task<Void, Never>?
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
        var downloads: [Downloaded]?
        var plays: [String: Int]?
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
            plays = stored.plays ?? [:]
            // Downloads used to live in Application Support, out of the
            // Files app's sight: move them to Documents/Downloads.
            Self.moveOldDownloads()
            // Keep only what is still on disk.
            downloads = (stored.downloads ?? []).filter {
                FileManager.default.fileExists(atPath: Self.downloadsDirectory.appendingPathComponent($0.file).path)
            }
        }
    }

    static var directory: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func save() {
        let stored = Stored(liked: liked, history: history, playlists: playlists, saved: saved,
                            downloads: downloads, plays: plays)
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
            // Keep the local ids of playlists already here (open screens point at them).
            let known = Dictionary(playlists.compactMap { p in p.remoteId.map { ($0, p.id) } },
                                   uniquingKeysWith: { first, _ in first })
            for i in remote.indices {
                if let remoteId = remote[i].remoteId, let id = known[remoteId] { remote[i].id = id }
            }
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
            self.error = String(localized: "Sync failed: \(error.friendly)")
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
                self.error = String(localized: "\(what) failed: \(error.friendly)")
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
        write(liking ? String(localized: "Liking \(track.title)") : String(localized: "Unliking \(track.title)")) { [catalog] in
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
        write(String(localized: "Disliking \(track.title)")) { [catalog] in
            try await catalog.rate(track.videoId, .dislike)
        }
    }

    func played(_ track: Track) {
        plays[track.videoId, default: 0] += 1
        history.removeAll { $0.videoId == track.videoId }
        history.insert(track, at: 0)
        if history.count > Self.historySize { history.removeLast(history.count - Self.historySize) }
        save()
    }

    func clearHistory() {
        history = []
        // Like the desktop player: counts go with the history.
        plays = [:]
        save()
    }

    // MARK: Lists

    func tracks(_ list: LocalList) -> [Track] {
        switch list {
        case .liked: liked
        case .history: history
        case .downloads: downloads.map(\.track)
        case let .playlist(id): playlists.first { $0.id == id }?.tracks ?? []
        }
    }

    func title(_ list: LocalList) -> String {
        switch list {
        case .liked: String(localized: "Liked songs")
        case .history: String(localized: "History")
        case .downloads: String(localized: "Downloads")
        case let .playlist(id): playlists.first { $0.id == id }?.name ?? String(localized: "Playlist")
        }
    }

    func remove(at offsets: IndexSet, from list: LocalList) {
        switch list {
        case .liked:
            let removed = offsets.map { liked[$0] }
            liked.remove(atOffsets: offsets)
            save()
            for track in removed {
                write(String(localized: "Unliking \(track.title)")) { [catalog] in
                    try await catalog.rate(track.videoId, .none)
                }
            }
        case .history:
            history.remove(atOffsets: offsets)
            save()
        case .downloads:
            for download in offsets.map({ downloads[$0] }) { removeDownload(download.track) }
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
                write(String(localized: "Removing \(entry.track.title)")) { [catalog] in
                    try await catalog.removeFromPlaylist(remoteId, entry.track.videoId,
                                                         setVideoId: entry.setVideoId)
                }
            }
        }
    }

    /// Reorders a playlist here and, for the account's own, on YouTube too
    /// (one move per song; a refusal puts the old order back).
    func move(from source: IndexSet, to destination: Int, in list: LocalList) {
        guard case let .playlist(id) = list, let i = playlists.firstIndex(where: { $0.id == id }) else { return }
        let before = playlists[i]
        playlists[i].tracks.move(fromOffsets: source, toOffset: destination)
        while playlists[i].setVideoIds.count < before.tracks.count { playlists[i].setVideoIds.append("") }
        playlists[i].setVideoIds.move(fromOffsets: source, toOffset: destination)
        save()
        guard let remoteId = before.remoteId, source.count == 1, let from = source.first else { return }
        let ids = playlists[i].setVideoIds
        let to = destination > from ? destination - 1 : destination
        guard ids.indices.contains(to), !ids[to].isEmpty else { return }
        let moved = ids[to]
        let successor = ids.indices.contains(to + 1) && !ids[to + 1].isEmpty ? ids[to + 1] : nil
        write(String(localized: "Moving the song")) { [catalog] in
            try await catalog.moveInPlaylist(remoteId, setVideoId: moved, before: successor)
        } undo: { [self] in
            guard let j = playlists.firstIndex(where: { $0.id == id }) else { return }
            playlists[j] = before
            save()
        }
    }

    @discardableResult
    func createPlaylist(_ name: String, with track: Track? = nil) -> UUID {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let title = trimmed.isEmpty ? String(localized: "Playlist \(playlists.count + 1)") : trimmed
        let playlist = LocalPlaylist(name: title, tracks: track.map { [$0] } ?? [],
                                     setVideoIds: track.map { _ in [""] } ?? [])
        playlists.append(playlist)
        save()
        let id = playlist.id
        write(String(localized: "Creating \(title)")) { [self] in
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

    /// A playlist of these songs (the queue saved as one): one request to
    /// YouTube, then a sync for the entries' ids.
    func createPlaylist(_ name: String, tracks: [Track]) {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let title = trimmed.isEmpty ? String(localized: "Playlist \(playlists.count + 1)") : trimmed
        let playlist = LocalPlaylist(name: title, tracks: tracks, setVideoIds: tracks.map { _ in "" })
        playlists.append(playlist)
        save()
        let id = playlist.id
        write(String(localized: "Creating \(title)")) { [self] in
            let remoteId = try await catalog.createPlaylist(title, videoIds: tracks.map(\.videoId))
            guard let i = playlists.firstIndex(where: { $0.id == id }) else { return }
            playlists[i].remoteId = remoteId
            save()
            await sync()
        } undo: { [self] in
            playlists.removeAll { $0.id == id }
            save()
        }
    }

    /// Renames a playlist here and, for the account's own, on YouTube too.
    func rename(_ id: UUID, to name: String) {
        let title = name.trimmingCharacters(in: .whitespaces)
        guard !title.isEmpty, let i = playlists.firstIndex(where: { $0.id == id }) else { return }
        let before = playlists[i]
        playlists[i].name = title
        save()
        guard let remoteId = before.remoteId else { return }
        write(String(localized: "Renaming \(before.name)")) { [catalog] in
            try await catalog.renamePlaylist(remoteId, to: title)
        } undo: { [self] in
            guard let j = playlists.firstIndex(where: { $0.id == id }) else { return }
            playlists[j].name = before.name
            save()
        }
    }

    /// Deletes playlists here and, for the account's own, on YouTube too.
    func deletePlaylists(at offsets: IndexSet) {
        let removed = offsets.map { playlists[$0] }
        playlists.remove(atOffsets: offsets)
        save()
        for playlist in removed {
            guard let remoteId = playlist.remoteId else { continue }
            write(String(localized: "Deleting \(playlist.name)")) { [catalog] in
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
        write(String(localized: "Adding \(track.title)")) { [self] in
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

    // MARK: Downloads

    /// Documents/Downloads: the app's folder in the Files app ("On My
    /// iPhone → Yarek Music"), so the songs can be copied or shared.
    static var downloadsDirectory: URL {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let dir = documents.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static func moveOldDownloads() {
        let old = directory.appendingPathComponent("downloads", isDirectory: true)
        guard let files = try? FileManager.default.contentsOfDirectory(atPath: old.path) else { return }
        for name in files {
            try? FileManager.default.moveItem(at: old.appendingPathComponent(name),
                                              to: downloadsDirectory.appendingPathComponent(name))
        }
        try? FileManager.default.removeItem(at: old)
    }

    /// "ABBA - Dancing Queen [YkLLcIKhJ64].m4a": readable in Files, unique by id.
    static func fileName(_ track: Track) -> String {
        let bad = CharacterSet(charactersIn: "/\\:*?\"<>|")
        let base = "\(track.artist) - \(track.title)".components(separatedBy: bad).joined(separator: "_")
        return "\(base.prefix(100)) [\(track.videoId)].m4a"
    }

    func isDownloaded(_ videoId: String) -> Bool {
        downloads.contains { $0.track.videoId == videoId }
    }

    /// The song's file and what playing it needs, when it is on the phone.
    func download(of videoId: String) -> (url: URL, item: Downloaded)? {
        guard let item = downloads.first(where: { $0.track.videoId == videoId }) else { return nil }
        return (Self.downloadsDirectory.appendingPathComponent(item.file), item)
    }

    /// Saves the song's audio on the phone (queued behind the others).
    func download(_ track: Track) {
        download([track])
    }

    /// Saves songs on the phone one after another (one stream at a time,
    /// like the desktop player), in 1 MB ranges — what YouTube's servers
    /// answer reliably.
    func download(_ tracks: [Track]) {
        var seen = Set<String>()
        let fresh = tracks.filter {
            seen.insert($0.videoId).inserted && !isDownloaded($0.videoId) && !downloading.contains($0.videoId)
        }
        guard !fresh.isEmpty else { return }
        if worker == nil {
            batchDone = 0
            batchTotal = 0
        }
        queue += fresh
        batchTotal += fresh.count
        downloading.formUnion(fresh.map(\.videoId))
        if worker == nil {
            worker = Task { await work() }
        }
    }

    private func work() async {
        var failures = 0
        while !queue.isEmpty, !Task.isCancelled {
            let track = queue.removeFirst()
            do {
                try await fetchSong(track)
                failures = 0
            } catch is CancellationError {
                break
            } catch {
                self.error = String(localized: "Download failed: \(error.friendly)")
                // No connection: the rest would fail the same way.
                failures += 1
                if !Connectivity.shared.online || failures >= 3 {
                    queue = []
                    downloading = []
                }
            }
            downloading.remove(track.videoId)
            batchDone += 1
        }
        worker = nil
    }

    private func fetchSong(_ track: Track) async throws {
        // The studio song's sound, also for a clip (as it plays).
        let stream = try await Services.resolver.stream(await Pairs.shared.audioId(track))
        guard !stream.hls else { throw DownloadError.notADownloadableFile }
        let file = Self.fileName(track)
        let target = Self.downloadsDirectory.appendingPathComponent(file)
        let bytes = try await Self.fetch(stream.url, length: stream.contentLength, to: target)
        // At the end: a list downloaded whole keeps its order.
        downloads.removeAll { $0.track.videoId == track.videoId }
        downloads.append(Downloaded(track: track, file: file, bytes: bytes,
                                    duration: stream.duration, loudnessDb: stream.loudnessDb))
        save()
        await Covers.fetch(track)
        await keepLyrics(of: track, duration: stream.duration)
    }

    var downloadedIds: Set<String> { Set(downloads.map(\.track.videoId)) }

    /// A downloaded song's lyrics, kept with it for offline play.
    private func keepLyrics(of track: Track, duration: TimeInterval?) async {
        guard !KeptLyrics.contains(track.videoId) else { return }
        let duration = duration ?? track.durationSecs.map(Double.init)
        _ = try? await KeptLyrics.find(track, duration: duration, keep: downloadedIds)
    }

    /// Songs downloaded before covers and lyrics were kept with them get
    /// theirs (once online).
    func fetchMissingExtras() async {
        guard Connectivity.shared.online else { return }
        for item in downloads {
            if Covers.local(item.track.videoId) == nil { await Covers.fetch(item.track) }
            await keepLyrics(of: item.track, duration: item.duration)
        }
    }

    /// Stops what is downloading and forgets what was waiting.
    func cancelDownloads() {
        queue = []
        worker?.cancel()
        worker = nil
        downloading = []
        batchDone = 0
        batchTotal = 0
    }

    func removeDownload(_ track: Track) {
        guard let (url, _) = download(of: track.videoId) else { return }
        try? FileManager.default.removeItem(at: url)
        Covers.remove(track.videoId)
        KeptLyrics.remove(track.videoId)
        downloads.removeAll { $0.track.videoId == track.videoId }
        save()
    }

    /// Deletes every downloaded song.
    func removeAllDownloads() {
        cancelDownloads()
        for item in downloads {
            try? FileManager.default.removeItem(at: Self.downloadsDirectory.appendingPathComponent(item.file))
            Covers.remove(item.track.videoId)
            KeptLyrics.remove(item.track.videoId)
        }
        downloads = []
        save()
    }

    enum DownloadError: LocalizedError {
        case notADownloadableFile
        var errorDescription: String? {
            String(localized: "Age-restricted songs can't be saved for offline play")
        }
    }

    /// Fetches `url` to `target` range by range; returns its size.
    nonisolated private static func fetch(_ url: URL, length: Int?, to target: URL) async throws -> Int {
        let chunk = 1 << 20
        let partial = target.appendingPathExtension("part")
        FileManager.default.createFile(atPath: partial.path, contents: nil)
        let handle = try FileHandle(forWritingTo: partial)
        defer { try? handle.close() }
        var offset = 0
        while true {
            try Task.checkCancellation()
            var request = URLRequest(url: url)
            let end = length.map { min(offset + chunk, $0) - 1 } ?? offset + chunk - 1
            request.setValue("bytes=\(offset)-\(end)", forHTTPHeaderField: "Range")
            let (data, response) = try await URLSession.shared.data(for: request)
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard status == 206 || status == 200 else { throw YTMError.http("Download", status) }
            try handle.write(contentsOf: data)
            offset += data.count
            // Done: the known length reached, or (length unknown) a short range.
            if data.isEmpty { break }
            if let length {
                if offset >= length { break }
            } else if data.count < chunk {
                break
            }
        }
        try? FileManager.default.removeItem(at: target)
        try FileManager.default.moveItem(at: partial, to: target)
        return offset
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
        write(saving ? String(localized: "Following \(page.title)") : String(localized: "Unfollowing \(page.title)")) { [catalog] in
            try await catalog.subscribe(channel, on: saving)
        }
    }

    func deleteSaved(at offsets: IndexSet) {
        let removed = offsets.map { saved[$0] }
        saved.remove(atOffsets: offsets)
        save()
        for page in removed where page.kind == .artist {
            write(String(localized: "Unfollowing \(page.title)")) { [catalog] in
                try await catalog.subscribe(page.id, on: false)
            }
        }
    }
}
