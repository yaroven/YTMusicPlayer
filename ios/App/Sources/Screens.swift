import SwiftUI
import YTMKit

/// YouTube Music's Home: the logo and the account at the top, mood chips,
/// then shelves ("Quick picks" as a grid of tiles, videos wide, the rest as
/// cards), more of them loading as you scroll, over the colours of the
/// first picture.
struct HomeView: View {
    @State private var shelves: [Shelf] = []
    @State private var chips: [HomeChip] = []
    @State private var chip: HomeChip?
    @State private var continuation: String?
    @State private var visitor: String?
    @State private var loadingMore = false
    /// Signed in or not when `shelves` were loaded.
    @State private var loadedAs: Bool?
    @State private var error: Error?
    @Environment(Library.self) private var library
    @Environment(Account.self) private var account
    @Environment(Router.self) private var router

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 32) {
                VStack(alignment: .leading, spacing: 18) {
                    HStack {
                        YTLogo()
                        Spacer()
                        AvatarButton()
                    }
                    .padding(.horizontal)
                    if !chips.isEmpty {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) {
                                ForEach(chips) { c in
                                    ChipButton(title: c.title, selected: chip == c) {
                                        chip = chip == c ? nil : c
                                        Task { await load() }
                                    }
                                }
                            }
                            .padding(.horizontal)
                        }
                    }
                }
                if !Connectivity.shared.online, !library.downloads.isEmpty {
                    HStack(spacing: 12) {
                        Image(systemName: "wifi.slash").foregroundStyle(YT.secondary)
                        Text("You're offline. Your downloaded songs still play.")
                            .font(.subheadline)
                            .foregroundStyle(YT.secondary)
                        Spacer(minLength: 0)
                        OutlinePill(title: "Downloads") { router.openDownloads() }
                    }
                    .padding(.horizontal)
                }
                if chip == nil, !library.history.isEmpty {
                    ShelfView(shelf: Shelf(title: String(localized: "Listen again"),
                                           items: library.history.prefix(20).map(\.item)))
                }
                ForEach(shelves) { ShelfView(shelf: $0) }
                if continuation != nil {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .onAppear { Task { await loadMore() } }
                }
            }
            .padding(.top, 8)
            .padding(.bottom, 24)
        }
        .background(alignment: .top) {
            AmbientBackdrop(art: shelves.first?.items.first?.artwork ?? .none)
        }
        .overlay {
            if shelves.isEmpty {
                if let error {
                    if library.history.isEmpty {
                        ErrorView(error: error) { Task { await load() } }
                    }
                } else if library.history.isEmpty {
                    LoadingView()
                }
            }
        }
        // Back online after a failure: load again without being asked.
        .onChange(of: Connectivity.shared.cameBack) {
            if error != nil || shelves.isEmpty { Task { await load() } }
        }
        .hiddenNavigationBar()
        // Signed in, Home is the account's own.
        .task(id: account.signedIn) {
            if loadedAs != account.signedIn { await load() }
        }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            let feed = try await Services.catalog.homeFeed(params: chip?.params)
            shelves = feed.shelves
            if chip == nil || chips.isEmpty { chips = feed.chips }
            continuation = feed.continuation
            visitor = feed.visitor
            loadedAs = account.signedIn
            error = nil
        } catch {
            self.error = error
        }
    }

    /// The next shelves, when the bottom of the list comes into view.
    private func loadMore() async {
        guard let token = continuation, !loadingMore else { return }
        loadingMore = true
        defer { loadingMore = false }
        guard let more = try? await Services.catalog.moreHome(token, visitor: visitor) else { return }
        guard continuation == token else { return }
        shelves += more.shelves
        continuation = more.next
    }
}

extension Track {
    var item: Item {
        Item(kind: .song, browseId: videoId, title: title, subtitle: artist, thumbnail: nil, track: self)
    }
}

/// Search, with Explore (new releases, moods, charts) when nothing is
/// typed and YouTube Music's suggestions while typing.
struct SearchView: View {
    @State private var query = ""
    @State private var kind: SearchKind = .songs
    @State private var results: [Item] = []
    @State private var searched = ""
    @State private var searching = false
    @State private var error: Error?
    @State private var task: Task<Void, Never>?
    @State private var explore: [Shelf] = []
    @State private var exploreError: Error?
    @State private var suggestions: [String] = []
    @State private var suggestTask: Task<Void, Never>?
    @FocusState private var typing: Bool

    var body: some View {
        VStack(spacing: 0) {
            searchField
            Group {
                if typing, !suggestions.isEmpty, searched != query.trimmingCharacters(in: .whitespaces) {
                    List(suggestions, id: \.self) { suggestion in
                        Button {
                            query = suggestion
                            run()
                        } label: {
                            HStack(spacing: 14) {
                                Image(systemName: "magnifyingglass").foregroundStyle(YT.secondary)
                                Text(suggestion)
                                Spacer()
                                Image(systemName: "arrow.up.left").foregroundStyle(YT.secondary)
                            }
                        }
                        .listRowBackground(Color.clear)
                    }
                    .listStyle(.plain)
                } else if searched.isEmpty && results.isEmpty {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 32) {
                            ForEach(explore) { ShelfView(shelf: $0) }
                        }
                        .padding(.vertical)
                    }
                    .overlay {
                        if explore.isEmpty {
                            if let exploreError {
                                ErrorView(error: exploreError) { Task { await loadExplore() } }
                            } else {
                                LoadingView()
                            }
                        }
                    }
                } else {
                    resultsList
                }
            }
            .frame(maxHeight: .infinity)
        }
        .background(YT.background)
        .hiddenNavigationBar()
        .onChange(of: kind) { run() }
        .onChange(of: query) { _, text in suggest(text) }
        .task { if explore.isEmpty { await loadExplore() } }
        .onChange(of: Connectivity.shared.cameBack) {
            if explore.isEmpty { Task { await loadExplore() } }
            if error != nil { run() }
        }
    }

    /// YouTube Music's search bar: a dark rounded field.
    private var searchField: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass").foregroundStyle(YT.secondary)
            TextField("Search songs, albums, artists", text: $query)
                .focused($typing)
                .submitLabel(.search)
                .onSubmit { run() }
                .autocorrectionDisabled()
            if !query.isEmpty {
                Button {
                    query = ""
                    searched = ""
                    results = []
                    typing = true
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(YT.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 42)
        .background(YT.surface, in: RoundedRectangle(cornerRadius: 21))
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    private func loadExplore() async {
        do {
            explore = try await Services.catalog.explore()
            exploreError = nil
        } catch {
            exploreError = error
        }
    }

    private var resultsList: some View {
        List {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(SearchKind.allCases) { k in
                        ChipButton(title: String(localized: String.LocalizationValue(k.rawValue)), selected: kind == k) {
                            kind = k
                        }
                    }
                }
                .padding(.horizontal)
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
            if kind == .songs {
                let tracks = results.compactMap(\.track)
                ForEach(Array(tracks.enumerated()), id: \.offset) { i, track in
                    TrackRow(track: track, list: tracks, index: i)
                }
            } else {
                ForEach(results) { ItemRow(item: $0) }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .scrollDismissesKeyboard(.immediately)
        .overlay {
            if searching {
                LoadingView()
            } else if let error {
                ErrorView(error: error) { run() }
            } else if results.isEmpty, !searched.isEmpty {
                ContentUnavailableView.search(text: searched)
            }
        }
    }

    /// One request after the typing stops, not one per keystroke.
    private func suggest(_ text: String) {
        suggestTask?.cancel()
        let text = text.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            suggestions = []
            return
        }
        suggestTask = Task {
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            let found = (try? await Services.catalog.suggestions(text)) ?? []
            guard !Task.isCancelled else { return }
            suggestions = Array(found.prefix(8))
        }
    }

    private func run() {
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return }
        task?.cancel()
        suggestTask?.cancel()
        suggestions = []
        typing = false
        searched = text
        searching = true
        let kind = kind
        task = Task {
            do {
                let found = try await Services.catalog.search(text, kind: kind)
                guard !Task.isCancelled else { return }
                results = found
                error = nil
            } catch {
                guard !Task.isCancelled else { return }
                results = []
                self.error = error
            }
            searching = false
        }
    }
}

/// An album, artist or playlist: its picture big over its own colours, the
/// title, then save / play / shuffle, then the songs and shelves.
struct PageView: View {
    let id: String
    @State private var page: Page?
    @State private var error: Error?
    @State private var showAll = false
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library

    init(id: String) {
        self.id = id
        // Opened lately (or prefetched while the finger was down): at once.
        _page = State(initialValue: PageCache.shared.cached(id))
    }

    /// Artist pages show this many top songs until "Show all".
    private static let topSongs = 5

    var body: some View {
        Group {
            if let page {
                content(page)
            } else if let preview = PageCache.shared.preview(id) {
                // The card's title and art at once, the songs when they come.
                List {
                    header(title: preview.title, subtitle: preview.subtitle,
                           art: preview.thumbnail.map(ArtworkSource.url) ?? .none,
                           round: preview.kind == .artist, tracks: [], page: nil)
                    if let error {
                        ErrorView(error: error) { Task { await load() } }
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                    } else {
                        ForEach(0..<8, id: \.self) { _ in SkeletonRow() }
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                    }
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
            } else if let error {
                ErrorView(error: error) { Task { await load() } }
            } else {
                LoadingView()
            }
        }
        .background(alignment: .top) {
            AmbientBackdrop(art: (page?.thumbnail ?? PageCache.shared.preview(id)?.thumbnail).map(ArtworkSource.url) ?? .none,
                            height: 520)
        }
        .navigationBarTitleDisplayMode(.inline)
        .clearNavigationBar()
        .task {
            if page == nil {
                await load()
            } else {
                await loadRest()
            }
        }
        .onChange(of: Connectivity.shared.cameBack) {
            if page == nil { Task { await load() } }
        }
    }

    /// The first answer shows at once; a long playlist's other songs (or an
    /// artist's full top songs) arrive behind it.
    private func load() async {
        error = nil
        do {
            page = try await PageCache.shared.load(id)
        } catch {
            self.error = error
            return
        }
        await loadRest()
    }

    private func loadRest() async {
        // Whatever arrived is what opening it again shows at once.
        defer { if let page { PageCache.shared.remember(id, page) } }
        if let all = page?.allSongsId,
           let list = try? await Services.catalog.page(all),
           list.tracks.count > page?.tracks.count ?? 0 {
            page?.tracks = list.tracks
        }
        page?.allSongsId = nil
        var parts = 0
        while let token = page?.continuation, parts < 50, !Task.isCancelled {
            parts += 1
            guard let more = try? await Services.catalog.moreTracks(token) else { return }
            page?.tracks += more.tracks
            page?.continuation = more.next
        }
    }

    /// Art, title, subtitle, then save / play / shuffle (also before the
    /// songs are in).
    private func header(title: String, subtitle: String, art: ArtworkSource, round: Bool,
                        tracks: [Track], page: Page?) -> some View {
        VStack(spacing: 14) {
            Artwork(source: art, size: round ? 200 : 250, round: round)
                .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
            Text(title)
                .font(.system(size: 26, weight: .bold))
                .multilineTextAlignment(.center)
            if !subtitle.isEmpty {
                Text(subtitle)
                    .font(.system(size: 14))
                    .foregroundStyle(YT.secondary)
                    .multilineTextAlignment(.center)
            }
            HStack(spacing: 22) {
                if let page {
                    CircleIcon(icon: library.isSaved(page.id) ? "bookmark.fill" : "bookmark") {
                        library.toggleSaved(page)
                    }
                } else {
                    CircleIcon(icon: "bookmark") {}.disabled(true)
                }
                CircleIcon(icon: "play.fill", size: 64, filled: true) { player.playFrom(tracks, at: 0) }
                    .disabled(tracks.isEmpty)
                CircleIcon(icon: "shuffle") { player.playFrom(tracks.shuffled(), at: 0) }
                    .disabled(tracks.isEmpty)
                DownloadAllButton(tracks: tracks)
            }
            .padding(.top, 4)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
    }

    private func content(_ page: Page) -> some View {
        let shown = page.kind == .artist && !showAll ? Array(page.tracks.prefix(Self.topSongs)) : page.tracks
        let title = page.title.isEmpty ? PageCache.shared.preview(id)?.title ?? "" : page.title
        return List {
            if page.tracks.isEmpty && page.thumbnail == nil {
                // Explore categories ("Chill") are only shelves.
                Text(title)
                    .font(.system(size: 28, weight: .bold))
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            } else {
                header(title: title, subtitle: page.subtitle,
                       art: page.thumbnail.map(ArtworkSource.url) ?? .none,
                       round: page.kind == .artist, tracks: page.tracks, page: page)
            }

            if page.kind == .artist, !page.tracks.isEmpty {
                Text("Top songs")
                    .font(YT.sectionTitle)
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            }
            ForEach(Array(shown.enumerated()), id: \.offset) { i, track in
                TrackRow(track: track, list: page.tracks, index: i, number: page.kind == .album ? i + 1 : nil)
            }
            if shown.count < page.tracks.count {
                HStack {
                    Spacer()
                    OutlinePill(title: "Show all \(page.tracks.count) songs") {
                        withAnimation { showAll = true }
                    }
                    Spacer()
                }
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
            }

            ForEach(page.shelves) { shelf in
                ShelfView(shelf: shelf)
                    .padding(.vertical, 12)
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }
}

/// What the library list shows: everything, or one kind.
private enum LibraryFilter: String, CaseIterable, Identifiable {
    case playlists = "Playlists", songs = "Songs", albums = "Albums", artists = "Artists", downloads = "Downloads"
    var id: String { rawValue }
}

/// YouTube Music's Library: chips to narrow it, then liked music,
/// downloads, history, playlists and saved albums / artists as one list.
struct LibraryView: View {
    @Environment(Library.self) private var library
    @Environment(Account.self) private var account
    @State private var filter: LibraryFilter?
    @State private var naming = false
    @State private var newName = ""

    var body: some View {
        List {
            HStack {
                Text("Library").font(.system(size: 28, weight: .bold))
                Spacer()
                Button {
                    newName = ""
                    naming = true
                } label: {
                    Image(systemName: "plus").font(.system(size: 20, weight: .medium))
                }
                .buttonStyle(.plain)
                .padding(.trailing, 8)
                .accessibilityLabel("New playlist")
                AvatarButton()
            }
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(LibraryFilter.allCases) { f in
                        ChipButton(title: String(localized: String.LocalizationValue(f.rawValue)), selected: filter == f) {
                            filter = filter == f ? nil : f
                        }
                    }
                }
                .padding(.horizontal)
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)

            if filter == nil || filter == .songs {
                entry(.liked, title: account.signedIn ? "Liked music" : "Liked songs",
                      count: library.liked.count) { LikedArt() }
            }
            if filter == nil || filter == .downloads {
                entry(.downloads, title: "Downloads", count: library.downloads.count) {
                    IconArt(icon: "arrow.down.circle.fill")
                }
            }
            if filter == nil {
                entry(.history, title: "History", count: library.history.count) {
                    IconArt(icon: "clock.arrow.circlepath")
                }
            }
            if filter == nil || filter == .playlists {
                ForEach(library.playlists) { playlist in
                    entry(.playlist(playlist.id), title: LocalizedStringKey(playlist.name), count: playlist.tracks.count) {
                        Artwork(source: playlist.tracks.first.map { .video($0.videoId) } ?? .none, size: 56)
                    }
                    .swipeActions {
                        Button("Delete", role: .destructive) {
                            if let i = library.playlists.firstIndex(of: playlist) {
                                library.deletePlaylists(at: IndexSet(integer: i))
                            }
                        }
                    }
                }
            }
            ForEach(library.saved.filter(shows)) { saved in
                ItemRow(item: Item(kind: saved.kind, browseId: saved.id, title: saved.title,
                                   subtitle: saved.subtitle, thumbnail: saved.thumbnail))
                    .swipeActions {
                        Button("Remove", role: .destructive) {
                            if let i = library.saved.firstIndex(of: saved) {
                                library.deleteSaved(at: IndexSet(integer: i))
                            }
                        }
                    }
            }
            if library.syncing {
                HStack(spacing: 10) {
                    ProgressView()
                    Text("Syncing your library…").foregroundStyle(YT.secondary)
                }
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(YT.background)
        .hiddenNavigationBar()
        .refreshable { await library.sync() }
        .task {
            if library.needsFirstSync { await library.sync() }
        }
        .onChange(of: account.signedIn) { _, signedIn in
            if signedIn {
                Task { await library.sync() }
            } else {
                library.forgetAccount()
            }
        }
        .alert("Library", isPresented: Binding(get: { library.error != nil },
                                               set: { if !$0 { library.error = nil } })) {
            Button("OK") { library.error = nil }
        } message: {
            Text(library.error ?? "")
        }
        .alert("New playlist", isPresented: $naming) {
            TextField("Name", text: $newName)
            Button("Create") { library.createPlaylist(newName) }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func shows(_ saved: SavedPage) -> Bool {
        switch filter {
        case nil: true
        case .albums: saved.kind == .album
        case .artists: saved.kind == .artist
        case .playlists: saved.kind == .playlist
        default: false
        }
    }

    private func entry<Art: View>(_ list: LocalList, title: LocalizedStringKey, count: Int,
                                  @ViewBuilder art: () -> Art) -> some View {
        NavigationLink { TrackListView(list: list) } label: {
            HStack(spacing: 14) {
                art()
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).font(.system(size: 16)).lineLimit(1)
                    Text("\(count) songs").font(.system(size: 14)).foregroundStyle(YT.secondary)
                }
            }
        }
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
    }
}

/// "Liked music": YouTube Music's purple tile with a thumbs-up.
private struct LikedArt: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 5)
            .fill(LinearGradient(colors: [Color(red: 0.55, green: 0.45, blue: 0.95), Color(red: 0.9, green: 0.4, blue: 0.75)],
                                 startPoint: .top, endPoint: .bottom))
            .overlay { Image(systemName: "hand.thumbsup.fill").font(.system(size: 24)).foregroundStyle(.white) }
            .frame(width: 56, height: 56)
    }
}

private struct IconArt: View {
    let icon: String

    var body: some View {
        RoundedRectangle(cornerRadius: 5)
            .fill(YT.surface)
            .overlay { Image(systemName: icon).font(.system(size: 24)).foregroundStyle(.white) }
            .frame(width: 56, height: 56)
    }
}

/// How a list of songs is ordered (as the desktop player sorts its lists).
enum SongSort: String, CaseIterable, Identifiable {
    case standard = "Default order", title = "Title", artist = "Artist", mostPlayed = "Most played"
    var id: String { rawValue }

    func apply(_ tracks: [Track], plays: [String: Int]) -> [Track] {
        switch self {
        case .standard: tracks
        case .title: tracks.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
        case .artist: tracks.sorted {
            ($0.artist.lowercased(), $0.title.lowercased()) < ($1.artist.lowercased(), $1.title.lowercased())
        }
        case .mostPlayed: tracks.sorted { plays[$0.videoId, default: 0] > plays[$1.videoId, default: 0] }
        }
    }
}

/// Download every song of a list: ⬇, a ring while it runs (tap stops), ✓
/// when all are on the phone.
struct DownloadAllButton: View {
    let tracks: [Track]
    @Environment(Library.self) private var library

    var body: some View {
        let all = !tracks.isEmpty && tracks.allSatisfy { library.isDownloaded($0.videoId) }
        let running = tracks.contains { library.downloading.contains($0.videoId) }
        if all {
            CircleIcon(icon: "arrow.down.circle.fill") {}
        } else if running {
            ZStack {
                ProgressView().scaleEffect(1.6)
                CircleIcon(icon: "stop.fill", size: 36) { library.cancelDownloads() }
            }
            .frame(width: 44, height: 44)
        } else {
            CircleIcon(icon: "arrow.down") { library.download(tracks) }
                .disabled(tracks.isEmpty)
                .accessibilityLabel("Download all")
        }
    }
}

/// "Downloading 3 of 12" with a bar, while downloads run.
struct DownloadProgress: View {
    @Environment(Library.self) private var library

    var body: some View {
        if !library.downloading.isEmpty, library.batchTotal > 0 {
            VStack(alignment: .leading, spacing: 6) {
                Text("Downloading \(min(library.batchDone + 1, library.batchTotal)) of \(library.batchTotal)")
                    .font(.footnote)
                    .foregroundStyle(YT.secondary)
                ProgressView(value: Double(library.batchDone), total: Double(library.batchTotal))
                    .tint(.white)
            }
        }
    }
}

/// Liked songs, history, downloads or a playlist.
struct TrackListView: View {
    let list: LocalList
    @Environment(Library.self) private var library
    @Environment(Player.self) private var player
    @State private var sort = SongSort.standard
    @State private var renaming = false
    @State private var newName = ""
    @State private var confirmDeleteAll = false

    var body: some View {
        let original = library.tracks(list)
        let tracks = sort.apply(original, plays: library.plays)
        List {
            VStack(alignment: .leading, spacing: 10) {
                Text(library.title(list))
                    .font(.system(size: 26, weight: .bold))
                Text("\(tracks.count) songs")
                    .font(.system(size: 14))
                    .foregroundStyle(YT.secondary)
                if !tracks.isEmpty {
                    HStack(spacing: 20) {
                        CircleIcon(icon: "play.fill", size: 56, filled: true) { player.playFrom(tracks, at: 0) }
                        CircleIcon(icon: "shuffle") { player.playFrom(tracks.shuffled(), at: 0) }
                        if list != .downloads { DownloadAllButton(tracks: tracks) }
                    }
                    .padding(.top, 4)
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(SongSort.allCases) { s in
                                ChipButton(title: String(localized: String.LocalizationValue(s.rawValue)), selected: sort == s) {
                                    sort = s
                                }
                            }
                        }
                    }
                    .padding(.top, 6)
                }
                if list == .downloads { DownloadProgress() }
            }
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
            ForEach(Array(tracks.enumerated()), id: \.offset) { i, track in
                TrackRow(track: track, list: tracks, index: i,
                         plays: sort == .mostPlayed ? library.plays[track.videoId, default: 0] : nil)
            }
            .onDelete { offsets in
                // Edits name the songs' places in the list as stored, not as sorted.
                let stored = IndexSet(offsets.compactMap { original.firstIndex(of: tracks[$0]) })
                library.remove(at: stored, from: list)
            }
            .onMove(perform: isPlaylist && sort == .standard ? { library.move(from: $0, to: $1, in: list) } : nil)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(alignment: .top) {
            AmbientBackdrop(art: tracks.first.map { .video($0.videoId) } ?? .none)
        }
        .overlay {
            if tracks.isEmpty {
                ContentUnavailableView("No songs yet", systemImage: "music.note",
                                       description: Text("Long-press a song to like it or add it to a playlist."))
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .clearNavigationBar()
        .toolbar {
            if case let .playlist(id) = list {
                Menu {
                    Button("Rename", systemImage: "pencil") {
                        newName = library.title(list)
                        renaming = true
                    }
                    EditButton()
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .alert("Rename", isPresented: $renaming) {
                    TextField("Name", text: $newName)
                    Button("OK") { library.rename(id, to: newName) }
                    Button("Cancel", role: .cancel) {}
                }
            }
            if list == .history, !tracks.isEmpty {
                Button("Clear") { library.clearHistory() }
            }
            if list == .downloads {
                if !library.downloading.isEmpty {
                    Button("Stop") { library.cancelDownloads() }
                }
                if !tracks.isEmpty {
                    Button("Delete all") { confirmDeleteAll = true }
                }
            }
        }
        .confirmationDialog("Delete all downloads?", isPresented: $confirmDeleteAll, titleVisibility: .visible) {
            Button("Delete all", role: .destructive) { library.removeAllDownloads() }
        } message: {
            Text("The songs stay in your library; only the files on the phone go.")
        }
    }

    private var isPlaylist: Bool {
        if case .playlist = list { return true }
        return false
    }
}

/// The avatar's sheet: the Google account, playback, language, about.
struct SettingsSheet: View {
    @Environment(Library.self) private var library
    @Environment(Player.self) private var player
    @Environment(Account.self) private var account
    @Environment(\.dismiss) private var dismiss
    @State private var signingIn = false
    @State private var confirmSignOut = false

    var body: some View {
        @Bindable var player = player
        NavigationStack {
            List {
                Section {
                    HStack(spacing: 14) {
                        Group {
                            if let photo = account.photo {
                                AsyncImage(url: photo) { $0.resizable().scaledToFill() } placeholder: { YT.raised }
                            } else {
                                Image(systemName: "person.crop.circle.fill").resizable().foregroundStyle(YT.secondary)
                            }
                        }
                        .frame(width: 48, height: 48)
                        .clipShape(Circle())
                        VStack(alignment: .leading, spacing: 2) {
                            if let name = account.name {
                                Text(name).font(.headline)
                            } else {
                                Text(account.signedIn ? "Signed in" : "Not signed in").font(.headline)
                            }
                            Text(account.signedIn ? "Google account" : "Sign in for your library")
                                .font(.subheadline)
                                .foregroundStyle(YT.secondary)
                        }
                    }
                    if account.signedIn {
                        Button("Sync library", systemImage: "arrow.triangle.2.circlepath") {
                            Task { await library.sync() }
                        }
                        .disabled(library.syncing)
                        Button("Sign out", systemImage: "rectangle.portrait.and.arrow.right", role: .destructive) {
                            confirmSignOut = true
                        }
                    } else {
                        Button("Sign in with Google", systemImage: "person.crop.circle.badge.plus") { signingIn = true }
                    }
                    NavigationLink { PlaybackTestView() } label: {
                        Label("Test age-restricted playback", systemImage: "stethoscope")
                    }
                } footer: {
                    if library.syncing {
                        Text("Syncing your library…")
                    } else if account.signedIn {
                        Text("Your liked music, playlists and followed artists are this account's, and changes here go back to YouTube.")
                    } else {
                        Text("Needed for your library, age-restricted songs and your own YouTube Music. The password goes only to Google.")
                    }
                }
                Section("Playback") {
                    Toggle("Autoplay similar songs", isOn: $player.autoplay)
                    Toggle("Normalize volume", isOn: $player.normalize)
                    VStack(alignment: .leading) {
                        if player.crossfade < 0.5 {
                            Text("Crossfade: off (gapless)")
                        } else {
                            Text("Crossfade: \(Int(player.crossfade)) s")
                        }
                        Slider(value: $player.crossfade, in: 0...12, step: 1)
                    }
                }
                #if os(iOS)
                Section {
                    // iOS keeps each app's language in its own Settings page.
                    Link(destination: URL(string: UIApplication.openSettingsURLString)!) {
                        Label("Language", systemImage: "globe")
                    }
                } footer: {
                    Text("English or Ukrainian: Settings → YTM Player → Language.")
                }
                #endif
                Section {
                    LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "-")
                    let updater = Updater.shared
                    if let update = updater.available {
                        Button("Update to \(update.version)", systemImage: "arrow.down.app") { updater.open() }
                    } else {
                        Button {
                            Task { await updater.check() }
                        } label: {
                            HStack {
                                Label("Check for updates", systemImage: "arrow.triangle.2.circlepath")
                                Spacer()
                                if updater.checking {
                                    ProgressView()
                                } else if updater.upToDate {
                                    Text("Up to date").foregroundStyle(YT.secondary)
                                }
                            }
                        }
                    }
                    Link("Source code", destination: URL(string: "https://github.com/yaroven/YTMusicPlayer")!)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .sheet(isPresented: $signingIn) { SignInView() }
            .confirmationDialog("Sign out of Google?", isPresented: $confirmSignOut, titleVisibility: .visible) {
                Button("Sign out", role: .destructive) { account.signOut() }
            }
        }
    }
}
