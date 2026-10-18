import SwiftUI
import YTMKit

struct HomeView: View {
    @State private var shelves: [Shelf] = []
    /// Signed in or not when `shelves` were loaded.
    @State private var loadedAs: Bool?
    @State private var error: String?
    @Environment(Library.self) private var library
    @Environment(Account.self) private var account

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 28) {
                if !library.history.isEmpty {
                    ShelfView(shelf: Shelf(title: String(localized: "Listen again"), items: library.history.prefix(20).map(\.item)))
                }
                ForEach(shelves) { ShelfView(shelf: $0) }
            }
            .padding(.vertical)
        }
        .overlay {
            if shelves.isEmpty {
                if let error {
                    ContentUnavailableView {
                        Label("Can't load Home", systemImage: "wifi.exclamationmark")
                    } description: {
                        Text(error)
                    } actions: {
                        Button("Retry") { Task { await load() } }
                    }
                } else if library.history.isEmpty {
                    ProgressView()
                }
            }
        }
        .navigationTitle("Home")
        // Signed in, Home is the account's own.
        .task(id: account.signedIn) {
            if loadedAs != account.signedIn { await load() }
        }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            shelves = try await Services.catalog.home()
            loadedAs = account.signedIn
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

extension Track {
    var item: Item {
        Item(kind: .song, browseId: videoId, title: title, subtitle: artist, thumbnail: nil, track: self)
    }
}

/// Search, with Explore (charts, new releases, moods) when nothing is typed
/// and YouTube Music's suggestions while typing.
struct SearchView: View {
    @State private var query = ""
    @State private var kind: SearchKind = .songs
    @State private var results: [Item] = []
    @State private var searched = ""
    @State private var searching = false
    @State private var error: String?
    @State private var task: Task<Void, Never>?
    @State private var explore: [Shelf] = []
    @State private var suggestions: [String] = []
    @State private var suggestTask: Task<Void, Never>?
    @Environment(\.isSearching) private var isSearching

    var body: some View {
        Group {
            if !suggestions.isEmpty, searched != query.trimmingCharacters(in: .whitespaces) {
                List(suggestions, id: \.self) { suggestion in
                    Button {
                        query = suggestion
                        run()
                    } label: {
                        Label(suggestion, systemImage: "magnifyingglass")
                            .foregroundStyle(.primary)
                    }
                }
                .listStyle(.plain)
            } else if searched.isEmpty && results.isEmpty {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 28) {
                        ForEach(explore) { ShelfView(shelf: $0) }
                    }
                    .padding(.vertical)
                }
                .overlay { if explore.isEmpty { ProgressView() } }
            } else {
                resultsList
            }
        }
        .navigationTitle("Search")
        .searchable(text: $query, prompt: "Songs, albums, artists")
        .onSubmit(of: .search) { run() }
        .onChange(of: kind) { run() }
        .onChange(of: query) { _, text in suggest(text) }
        .task { if explore.isEmpty { explore = (try? await Services.catalog.explore()) ?? [] } }
    }

    private var resultsList: some View {
        List {
            Picker("Kind", selection: $kind) {
                ForEach(SearchKind.allCases) { Text(LocalizedStringKey($0.rawValue)).tag($0) }
            }
            .pickerStyle(.segmented)
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
        .overlay {
            if searching {
                ProgressView()
            } else if let error {
                ContentUnavailableView("Search failed", systemImage: "exclamationmark.triangle",
                                       description: Text(error))
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
                self.error = error.localizedDescription
            }
            searching = false
        }
    }
}

struct PageView: View {
    let id: String
    @State private var page: Page?
    @State private var error: String?
    @State private var showAll = false
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library

    /// Artist pages show this many top songs until "Show all".
    private static let topSongs = 5

    var body: some View {
        Group {
            if let page {
                content(page)
            } else if let error {
                ContentUnavailableView("Can't open", systemImage: "exclamationmark.triangle", description: Text(error))
            } else {
                ProgressView()
            }
        }
        .navigationTitle(page?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let page {
                Button {
                    library.toggleSaved(page)
                } label: {
                    Image(systemName: library.isSaved(page.id) ? "bookmark.fill" : "bookmark")
                }
            }
        }
        .task {
            guard page == nil else { return }
            do {
                page = try await Services.catalog.page(id)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func content(_ page: Page) -> some View {
        let shown = page.kind == .artist && !showAll ? Array(page.tracks.prefix(Self.topSongs)) : page.tracks
        return List {
            VStack(spacing: 12) {
                Artwork(source: page.thumbnail.map(ArtworkSource.url) ?? .none, size: 220, round: page.kind == .artist)
                    .shadow(radius: 16)
                Text(page.title)
                    .font(.title2.bold())
                    .multilineTextAlignment(.center)
                Text(page.subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                HStack {
                    Button { player.play(page.tracks) } label: {
                        Label("Play", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button { player.play(page.tracks.shuffled()) } label: {
                        Label("Shuffle", systemImage: "shuffle").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                .disabled(page.tracks.isEmpty)
            }
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)

            Section {
                ForEach(Array(shown.enumerated()), id: \.offset) { i, track in
                    TrackRow(track: track, list: page.tracks, index: i, number: page.kind == .album ? i + 1 : nil)
                }
                if shown.count < page.tracks.count {
                    Button("Show all \(page.tracks.count) songs") {
                        withAnimation { showAll = true }
                    }
                }
            } header: {
                if page.kind == .artist, !page.tracks.isEmpty { Text("Top songs") }
            }

            ForEach(page.shelves) { shelf in
                Section {
                    ShelfView(shelf: shelf, showTitle: false)
                        .listRowInsets(EdgeInsets())
                        .listRowSeparator(.hidden)
                } header: {
                    Text(shelf.title)
                }
            }
        }
        .listStyle(.plain)
    }
}

struct LibraryView: View {
    @Environment(Library.self) private var library
    @Environment(Player.self) private var player
    @Environment(Account.self) private var account
    @State private var naming = false
    @State private var newName = ""
    @State private var signingIn = false
    @State private var confirmSignOut = false

    var body: some View {
        @Bindable var player = player
        List {
            Section {
                NavigationLink { TrackListView(list: .liked) } label: {
                    Label(account.signedIn ? "Liked music" : "Liked songs", systemImage: "heart.fill")
                        .badge(library.liked.count)
                }
                NavigationLink { TrackListView(list: .history) } label: {
                    Label("History", systemImage: "clock")
                }
                NavigationLink { TrackListView(list: .downloads) } label: {
                    Label("Downloads", systemImage: "arrow.down.circle")
                        .badge(library.downloads.count)
                }
            }
            Section("Playlists") {
                ForEach(library.playlists) { playlist in
                    NavigationLink { TrackListView(list: .playlist(playlist.id)) } label: {
                        Label(playlist.name, systemImage: "music.note.list")
                            .badge(playlist.tracks.count)
                    }
                }
                .onDelete { library.deletePlaylists(at: $0) }
                Button("New playlist", systemImage: "plus") {
                    newName = ""
                    naming = true
                }
            }
            if !library.saved.isEmpty {
                Section("Albums, artists, playlists") {
                    ForEach(library.saved) { saved in
                        NavigationLink(value: Route.page(saved.id)) {
                            HStack(spacing: 12) {
                                Artwork(source: saved.thumbnail.map(ArtworkSource.url) ?? .none, size: 48,
                                        round: saved.kind == .artist)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(saved.title).lineLimit(1)
                                    Text(saved.subtitle)
                                        .font(.subheadline)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                            }
                        }
                    }
                    .onDelete { library.deleteSaved(at: $0) }
                }
            }
            Section {
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
            } header: {
                Text("Google account")
            } footer: {
                if library.syncing {
                    Text("Syncing your library…")
                } else if account.signedIn {
                    if let name = account.name {
                        Text("Signed in as \(name). Your liked music, playlists and followed artists are this account's, and changes here go back to YouTube.")
                    } else {
                        Text("Signed in. Your liked music, playlists and followed artists are this account's, and changes here go back to YouTube.")
                    }
                } else {
                    Text("Needed for your library, age-restricted songs and your own YouTube Music. The password goes only to Google.")
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
            Section {
                LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "-")
                Link("Source code", destination: URL(string: "https://github.com/yaroven/YTMusicPlayer")!)
            }
        }
        .navigationTitle("Library")
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
        .sheet(isPresented: $signingIn) { SignInView() }
        .confirmationDialog("Sign out of Google?", isPresented: $confirmSignOut, titleVisibility: .visible) {
            Button("Sign out", role: .destructive) { account.signOut() }
        }
        .alert("New playlist", isPresented: $naming) {
            TextField("Name", text: $newName)
            Button("Create") { library.createPlaylist(newName) }
            Button("Cancel", role: .cancel) {}
        }
    }
}

struct TrackListView: View {
    let list: LocalList
    @Environment(Library.self) private var library
    @Environment(Player.self) private var player

    var body: some View {
        let tracks = library.tracks(list)
        List {
            if !tracks.isEmpty {
                HStack {
                    Button { player.play(tracks) } label: {
                        Label("Play", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button { player.play(tracks.shuffled()) } label: {
                        Label("Shuffle", systemImage: "shuffle").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                .listRowSeparator(.hidden)
            }
            ForEach(Array(tracks.enumerated()), id: \.offset) { i, track in
                TrackRow(track: track, list: tracks, index: i)
            }
            .onDelete { library.remove(at: $0, from: list) }
            .onMove(perform: isPlaylist ? { library.move(from: $0, to: $1, in: list) } : nil)
        }
        .listStyle(.plain)
        .overlay {
            if tracks.isEmpty {
                ContentUnavailableView("No songs yet", systemImage: "music.note",
                                       description: Text("Long-press a song to like it or add it to a playlist."))
            }
        }
        .navigationTitle(library.title(list))
        .toolbar {
            if isPlaylist { EditButton() }
            if list == .history, !tracks.isEmpty {
                Button("Clear") { library.clearHistory() }
            }
        }
    }

    private var isPlaylist: Bool {
        if case .playlist = list { return true }
        return false
    }
}
