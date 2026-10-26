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
                    HomeHeader()
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

/// Home's and Explore's header: the logo, search and the account.
struct HomeHeader: View {
    @Environment(Router.self) private var router

    var body: some View {
        HStack(spacing: 20) {
            YTLogo()
            Spacer()
            Button { router.open(.search) } label: {
                Image(systemName: "magnifyingglass").font(.system(size: 20, weight: .medium))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Search")
            AvatarButton()
        }
        .padding(.horizontal)
    }
}

/// The Explore tab: the header, then YouTube Music's Explore shelves (new
/// releases, charts, moods).
struct ExploreView: View {
    @State private var explore: [Shelf] = []
    @State private var error: Error?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 32) {
                HomeHeader()
                ForEach(explore) { ShelfView(shelf: $0) }
            }
            .padding(.top, 8)
            .padding(.bottom, 24)
        }
        .background(YT.background)
        .overlay {
            if explore.isEmpty {
                if let error {
                    ErrorView(error: error) { Task { await load() } }
                } else {
                    LoadingView()
                }
            }
        }
        .hiddenNavigationBar()
        .task { if explore.isEmpty { await load() } }
        .onChange(of: Connectivity.shared.cameBack) {
            if explore.isEmpty { Task { await load() } }
        }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            explore = try await Services.catalog.explore()
            error = nil
        } catch {
            self.error = error
        }
    }
}

/// Search, opened from a header's magnifier: ←, the field, the songs played
/// lately before anything is typed, YouTube Music's suggestions while typing.
struct SearchView: View {
    @State private var query = ""
    @State private var kind: SearchKind = .songs
    @State private var results: [Item] = []
    @State private var searched = ""
    @State private var searching = false
    @State private var error: Error?
    @State private var task: Task<Void, Never>?
    @State private var suggestions: [String] = []
    @Environment(Library.self) private var library
    @Environment(\.dismiss) private var dismiss
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
                    let recent = Array(library.history.prefix(12))
                    List {
                        if !recent.isEmpty {
                            Text("Recently played")
                                .font(.system(size: 17, weight: .bold))
                                .listRowBackground(Color.clear)
                                .listRowSeparator(.hidden)
                        }
                        ForEach(Array(recent.enumerated()), id: \.offset) { i, track in
                            TrackRow(track: track, list: recent, index: i)
                        }
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .scrollDismissesKeyboard(.immediately)
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
        .onAppear { if searched.isEmpty { typing = true } }
        .onChange(of: Connectivity.shared.cameBack) {
            if error != nil { run() }
        }
    }

    /// YouTube Music's search bar: ←, a dark rounded field.
    private var searchField: some View {
        HStack(spacing: 6) {
        Button { dismiss() } label: {
            Image(systemName: "chevron.left").font(.system(size: 20, weight: .semibold)).frame(width: 40, height: 40)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Back")
        HStack(spacing: 10) {
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
        .padding(.horizontal, 16)
        .frame(height: 44)
        .background(YT.surface, in: Capsule())
        }
        .padding(.leading, 6)
        .padding(.trailing)
        .padding(.vertical, 8)
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
            if (page?.kind ?? PageCache.shared.preview(id)?.kind) != .artist {
                AmbientBackdrop(art: (page?.thumbnail ?? PageCache.shared.preview(id)?.thumbnail).map(ArtworkSource.url) ?? .none,
                                height: 520)
            }
        }
        .background(YT.background)
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

    /// Art, title, subtitle, then YouTube Music's circles: download, save,
    /// play, share and ⋮ (also before the songs are in). Artists get their
    /// own header.
    @ViewBuilder
    private func header(title: String, subtitle: String, art: ArtworkSource, round: Bool,
                        tracks: [Track], page: Page?) -> some View {
        if round {
            artistHeader(title: title, subtitle: subtitle, art: art, tracks: tracks, page: page)
        } else {
            VStack(spacing: 14) {
                Artwork(source: art, size: 220)
                    .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
                Text(title)
                    .font(.system(size: 28, weight: .bold))
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                // A playlist says how many songs it has, next to YouTube's line.
                let songs = page?.kind == .playlist && !tracks.isEmpty ? String(localized: "\(tracks.count) songs") : ""
                let shown = subtitle.isEmpty ? songs : songs.isEmpty ? subtitle : "\(subtitle) • \(songs)"
                if !shown.isEmpty {
                    Text(shown)
                        .font(.system(size: 14))
                        .foregroundStyle(YT.secondary)
                        .multilineTextAlignment(.center)
                }
                HStack(spacing: 16) {
                    DownloadAllButton(tracks: tracks)
                    if let page {
                        let saved = library.isSaved(page.id)
                        CircleIcon(icon: saved ? "bookmark.fill" : "bookmark") { library.toggleSaved(page) }
                            .accessibilityLabel(saved ? "Remove from library" : "Save")
                    } else {
                        CircleIcon(icon: "bookmark") {}.disabled(true)
                    }
                    CircleIcon(icon: "play.fill", size: 64, filled: true) { player.playFrom(tracks, at: 0) }
                        .disabled(tracks.isEmpty)
                    if let page {
                        ShareLink(item: URL(string: "https://music.youtube.com/browse/\(page.id)")!) {
                            Image(systemName: "arrowshape.turn.up.right")
                                .font(.system(size: 20, weight: .semibold))
                                .foregroundStyle(.white)
                                .frame(width: 48, height: 48)
                                .background(YT.raised, in: Circle())
                        }
                        .accessibilityLabel("Share")
                    }
                    Menu {
                        Button("Shuffle", systemImage: "shuffle") { player.playFrom(tracks.shuffled(), at: 0) }
                        if let first = tracks.first {
                            Button("Start mix", systemImage: "dot.radiowaves.left.and.right") { player.startRadio(first) }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .rotationEffect(.degrees(90))
                            .font(.system(size: 20, weight: .semibold))
                            .foregroundStyle(.white)
                            .frame(width: 48, height: 48)
                            .background(YT.raised, in: Circle())
                    }
                    .disabled(tracks.isEmpty)
                    .accessibilityLabel("More")
                }
                .padding(.top, 4)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
        }
    }

    /// An artist: the picture across the whole width with the name over its
    /// bottom, then Subscribe, radio and play.
    private func artistHeader(title: String, subtitle: String, art: ArtworkSource, tracks: [Track], page: Page?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            GeometryReader { geo in
                ZStack(alignment: .bottomLeading) {
                    if case let .url(url) = art {
                        AsyncImage(url: URL(string: heroURL(url.absoluteString, width: 1080, height: 1028))) { image in
                            image.resizable().scaledToFill()
                        } placeholder: { YT.surface }
                        .frame(width: geo.size.width, height: geo.size.height)
                        .clipped()
                    }
                    LinearGradient(stops: [.init(color: .black.opacity(0.35), location: 0), .init(color: .clear, location: 0.25),
                                           .init(color: .clear, location: 0.6), .init(color: YT.background, location: 1)],
                                   startPoint: .top, endPoint: .bottom)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title).font(.system(size: 40, weight: .bold)).lineLimit(2)
                        if !subtitle.isEmpty {
                            Text(subtitle).font(.system(size: 14)).foregroundStyle(YT.secondary).lineLimit(1)
                        }
                    }
                    .padding(.horizontal, 16)
                }
            }
            .aspectRatio(1.05, contentMode: .fit)
            HStack(spacing: 12) {
                if let page {
                    let saved = library.isSaved(page.id)
                    Button { library.toggleSaved(page) } label: {
                        Text(saved ? "Subscribed" : "Subscribe")
                            .font(.system(size: 15, weight: .medium))
                            .foregroundStyle(saved ? .white : .black)
                            .padding(.horizontal, 18)
                            .frame(height: 40)
                            .background(saved ? YT.raised : .white, in: Capsule())
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
                CircleIcon(icon: "dot.radiowaves.left.and.right", size: 40) {
                    if let first = tracks.first { player.startRadio(first) }
                }
                .disabled(tracks.isEmpty)
                .accessibilityLabel("Start mix")
                CircleIcon(icon: "play.fill", size: 56, filled: true) { player.playFrom(tracks, at: 0) }
                    .disabled(tracks.isEmpty)
            }
            .padding(16)
        }
        .listRowInsets(EdgeInsets())
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
                HStack {
                    Text("Top songs").font(.system(size: 24, weight: .bold))
                    Spacer()
                    OutlinePill(title: "Play all") { player.playFrom(page.tracks, at: 0) }
                }
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
            }
            ForEach(Array(shown.enumerated()), id: \.offset) { i, track in
                TrackRow(track: track, list: page.tracks, index: i, number: page.kind == .album ? i + 1 : nil,
                         art: page.kind == .artist ? 56 : 48)
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
        // An artist's picture reaches the top, under the bar.
        .ignoresSafeArea(edges: page.kind == .artist ? .top : [])
    }
}

/// An artist's picture big enough for the whole width, cropped by Google.
func heroURL(_ url: String, width: Int, height: Int) -> String {
    guard url.contains("googleusercontent.com"), let cut = url.range(of: "=w", options: .backwards) else { return url }
    return String(url[..<cut.lowerBound]) + "=w\(width)-h\(height)-p-l90-rj"
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
    @Environment(Router.self) private var router
    @Environment(Player.self) private var player
    @State private var filter: LibraryFilter?
    @AppStorage("library.byName") private var byName = false
    @State private var showHistory = false
    @State private var naming = false
    @State private var newName = ""

    var body: some View {
        List {
            HStack(spacing: 18) {
                Text("Library").font(.system(size: 28, weight: .bold))
                Spacer()
                Button { showHistory = true } label: {
                    Image(systemName: "clock.arrow.circlepath").font(.system(size: 20, weight: .medium))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("History")
                Button { router.open(.search) } label: {
                    Image(systemName: "magnifyingglass").font(.system(size: 20, weight: .medium))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Search")
                Button {
                    newName = ""
                    naming = true
                } label: {
                    Image(systemName: "plus").font(.system(size: 20, weight: .medium))
                }
                .buttonStyle(.plain)
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

            // "Recent activity ▾": YouTube Music's order, or by name.
            Menu {
                Picker("", selection: $byName) {
                    Text("Recent activity").tag(false)
                    Text("A–Z").tag(true)
                }
            } label: {
                HStack(spacing: 4) {
                    Text(byName ? "A–Z" : "Recent activity").font(.system(size: 15, weight: .medium))
                    Image(systemName: "chevron.down").font(.system(size: 12, weight: .semibold))
                }
                .foregroundStyle(.white)
            }
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
            if filter == nil || filter == .playlists {
                ForEach(byName ? library.playlists.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
                               : library.playlists) { playlist in
                    entry(.playlist(playlist.id), title: LocalizedStringKey(playlist.name), count: playlist.tracks.count, playlist: true) {
                        Artwork(source: playlist.tracks.first.map { .video($0.videoId) } ?? .none, size: 64)
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
            ForEach((byName ? library.saved.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
                             : library.saved).filter(shows)) { saved in
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
        .safeAreaInset(edge: .bottom) { Color.clear.frame(height: 64) }
        .overlay(alignment: .bottomTrailing) { shuffleAll }
        .background(YT.background)
        .hiddenNavigationBar()
        .navigationDestination(isPresented: $showHistory) { TrackListView(list: .history) }
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

    /// "Shuffle all": every liked and playlist song, shuffled.
    @ViewBuilder private var shuffleAll: some View {
        let everything = (library.liked + library.playlists.flatMap(\.tracks)).reduce(into: [Track]()) { all, track in
            if !all.contains(where: { $0.videoId == track.videoId }) { all.append(track) }
        }
        if !everything.isEmpty {
            Button { player.playFrom(everything.shuffled(), at: 0) } label: {
                Label("Shuffle all", systemImage: "shuffle")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(.black)
                    .padding(.horizontal, 18)
                    .frame(height: 46)
                    .background(.white, in: Capsule())
                    .shadow(color: .black.opacity(0.4), radius: 8, y: 4)
            }
            .buttonStyle(.plain)
            .padding(16)
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

    private func entry<Art: View>(_ list: LocalList, title: LocalizedStringKey, count: Int, playlist: Bool = false,
                                  @ViewBuilder art: () -> Art) -> some View {
        NavigationLink { TrackListView(list: list) } label: {
            HStack(spacing: 16) {
                art()
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).font(.system(size: 17)).lineLimit(1)
                    let songs = String(localized: "\(count) songs")
                    Text(playlist ? String(localized: "Playlist • \(songs)") : songs)
                        .font(.system(size: 14)).foregroundStyle(YT.secondary)
                }
            }
        }
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
    }
}

/// "Liked music": YouTube Music's purple tile with a thumbs-up.
private struct LikedArt: View {
    var size: CGFloat = 64

    var body: some View {
        RoundedRectangle(cornerRadius: 5)
            .fill(LinearGradient(colors: [Color(red: 0.55, green: 0.45, blue: 0.95), Color(red: 0.9, green: 0.4, blue: 0.75)],
                                 startPoint: .top, endPoint: .bottom))
            .overlay { Image(systemName: "hand.thumbsup.fill").font(.system(size: size * 0.4)).foregroundStyle(.white) }
            .frame(width: size, height: size)
    }
}

private struct IconArt: View {
    let icon: String
    var size: CGFloat = 64

    var body: some View {
        RoundedRectangle(cornerRadius: 5)
            .fill(YT.surface)
            .overlay { Image(systemName: icon).font(.system(size: size * 0.4)).foregroundStyle(.white) }
            .frame(width: size, height: size)
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
                ProgressView().scaleEffect(1.7)
                CircleIcon(icon: "stop.fill", size: 40) { library.cancelDownloads() }
            }
            .frame(width: 48, height: 48)
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
            // YouTube Music's list header: the picture, the name, "N songs", ⬇ ▶ shuffle.
            VStack(spacing: 10) {
                Group {
                    switch list {
                    case .liked: LikedArt(size: 200)
                    case .downloads: IconArt(icon: "arrow.down.circle.fill", size: 200)
                    case .history: IconArt(icon: "clock.arrow.circlepath", size: 200)
                    default: Artwork(source: tracks.first.map { .video($0.videoId) } ?? .none, size: 200)
                    }
                }
                .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
                .padding(.bottom, 6)
                Text(library.title(list))
                    .font(.system(size: 26, weight: .bold))
                    .multilineTextAlignment(.center)
                Text("\(tracks.count) songs")
                    .font(.system(size: 14))
                    .foregroundStyle(YT.secondary)
                if !tracks.isEmpty {
                    HStack(spacing: 28) {
                        if list != .downloads { DownloadAllButton(tracks: tracks) }
                        CircleIcon(icon: "play.fill", size: 64, filled: true) { player.playFrom(tracks, at: 0) }
                        CircleIcon(icon: "shuffle") { player.playFrom(tracks.shuffled(), at: 0) }
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
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
            // Playlists whose every song is on the phone, as YouTube Music
            // lists them under Downloads; then all the songs.
            if list == .downloads {
                let whole = library.playlists.filter { p in
                    !p.tracks.isEmpty && p.tracks.allSatisfy { library.isDownloaded($0.videoId) }
                }
                if !whole.isEmpty {
                    Text("Downloaded playlists")
                        .font(.system(size: 20, weight: .bold))
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                    ForEach(whole) { playlist in
                        NavigationLink { TrackListView(list: .playlist(playlist.id)) } label: {
                            HStack(spacing: 16) {
                                Artwork(source: playlist.tracks.first.map { .video($0.videoId) } ?? .none, size: 56)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(playlist.name).font(.system(size: 17)).lineLimit(1)
                                    let songs = String(localized: "\(playlist.tracks.count) songs")
                                    Text(String(localized: "Playlist • \(songs)"))
                                        .font(.system(size: 14)).foregroundStyle(YT.secondary)
                                }
                            }
                        }
                        .listRowBackground(Color.clear)
                    }
                    Text("Songs")
                        .font(.system(size: 20, weight: .bold))
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                }
            }
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
                    Text("English or Ukrainian: Settings → Yarek Music → Language.")
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
