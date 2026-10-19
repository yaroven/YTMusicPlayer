import SwiftUI
import YTMKit

struct RootView: View {
    @Environment(Player.self) private var player
    @Environment(Router.self) private var router

    init() {
        #if os(iOS)
        // YouTube Music's tab bar: black, white when selected, grey otherwise.
        let bar = UITabBarAppearance()
        bar.configureWithOpaqueBackground()
        bar.backgroundColor = UIColor(YT.background)
        bar.shadowColor = UIColor.white.withAlphaComponent(0.08)
        for item in [bar.stackedLayoutAppearance, bar.inlineLayoutAppearance, bar.compactInlineLayoutAppearance] {
            item.normal.iconColor = UIColor(YT.secondary)
            item.normal.titleTextAttributes = [.foregroundColor: UIColor(YT.secondary)]
            item.selected.iconColor = .white
            item.selected.titleTextAttributes = [.foregroundColor: UIColor.white]
        }
        UITabBar.appearance().standardAppearance = bar
        UITabBar.appearance().scrollEdgeAppearance = bar
        #endif
    }

    var body: some View {
        @Bindable var router = router
        @Bindable var player = player
        TabView(selection: $router.tab) {
            NavigationStack(path: $router.home) { HomeView().routes() }
                .miniPlayer()
                .tabItem { Label("Home", systemImage: router.tab == .home ? "house.fill" : "house") }
                .tag(AppTab.home)
            NavigationStack(path: $router.search) { SearchView().routes() }
                .miniPlayer()
                .tabItem { Label("Search", systemImage: "magnifyingglass") }
                .tag(AppTab.search)
            NavigationStack(path: $router.library) { LibraryView().routes() }
                .miniPlayer()
                .tabItem { Label("Library", systemImage: router.tab == .library ? "bookmark.fill" : "bookmark") }
                .tag(AppTab.library)
        }
        .sheet(isPresented: $player.showNowPlaying) {
            NowPlayingView()
                .presentationDragIndicator(.hidden)
                .overlay(alignment: .bottom) { NoticeToast() }
        }
        .safeAreaInset(edge: .top, spacing: 0) { ConnectionBanner() }
        .overlay(alignment: .bottom) { NoticeToast() }
        .animation(.easeInOut(duration: 0.25), value: Notice.shared.text)
    }
}

extension View {
    func routes() -> some View {
        navigationDestination(for: Route.self) { route in
            switch route {
            case let .page(id): PageView(id: id)
            case .downloads: TrackListView(list: .downloads)
            }
        }
    }

    func miniPlayer() -> some View {
        safeAreaInset(edge: .bottom, spacing: 0) { MiniPlayer() }
    }

    func trackMenu(_ track: Track) -> some View {
        contextMenu { TrackMenuItems(track: track) }
    }
}

/// A song's actions, for its long-press menu and its ⋮ button.
struct TrackMenuItems: View {
    let track: Track
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router

    var body: some View {
        Button("Start radio", systemImage: "dot.radiowaves.left.and.right") { player.startRadio(track) }
        Button("Play next", systemImage: "text.insert") { player.playNext(track) }
        Button("Add to queue", systemImage: "text.append") { player.addToQueue(track) }
        Divider()
        let liked = library.isLiked(track)
        Button(liked ? "Remove from liked" : "Like", systemImage: liked ? "hand.thumbsup.fill" : "hand.thumbsup") {
            library.toggleLike(track)
        }
        Menu("Add to playlist", systemImage: "text.badge.plus") {
            ForEach(library.playlists) { playlist in
                Button(playlist.name) { library.add(track, to: playlist.id) }
            }
            Button("New playlist", systemImage: "plus") { library.createPlaylist("", with: track) }
        }
        if library.isDownloaded(track.videoId) {
            Button("Remove download", systemImage: "trash") { library.removeDownload(track) }
        } else if library.downloading.contains(track.videoId) {
            Button("Downloading…", systemImage: "arrow.down.circle") {}.disabled(true)
        } else {
            Button("Download", systemImage: "arrow.down.circle") { library.download(track) }
        }
        Button("Dislike", systemImage: "hand.thumbsdown") { player.dislike(track) }
        Button("Go to artist", systemImage: "person") {
            player.showNowPlaying = false
            router.openArtist(of: track)
        }
        ShareLink(item: URL(string: "https://music.youtube.com/watch?v=\(track.videoId)")!) {
            Label("Share", systemImage: "square.and.arrow.up")
        }
    }
}

/// YouTube Music's "E": explicit lyrics (18+).
struct ExplicitBadge: View {
    var body: some View {
        Text("E")
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(.black)
            .frame(width: 14, height: 14)
            .background(YT.secondary, in: RoundedRectangle(cornerRadius: 2))
            .accessibilityLabel("Explicit")
    }
}

/// A song in a list: art (or its number on an album), title, artist, ⋮.
struct TrackRow: View {
    let track: Track
    let list: [Track]
    let index: Int
    /// Album pages number their songs instead of showing art.
    var number: Int?
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library

    var body: some View {
        HStack(spacing: 14) {
            Button { player.playFrom(list, at: index) } label: {
                HStack(spacing: 14) {
                    if let number {
                        Text("\(number)")
                            .font(.system(size: 15).monospacedDigit())
                            .foregroundStyle(isCurrent ? .white : YT.secondary)
                            .frame(width: 24)
                    } else {
                        Artwork(source: .video(track.videoId), size: 48)
                            .overlay {
                                if isCurrent {
                                    Image(systemName: player.isPlaying ? "waveform" : "play.fill")
                                        .font(.system(size: 18, weight: .bold))
                                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                                        .background(.black.opacity(0.5))
                                }
                            }
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        Text(track.title)
                            .font(.system(size: 16))
                            .lineLimit(1)
                        HStack(spacing: 5) {
                            if track.explicit { ExplicitBadge() }
                            if library.isDownloaded(track.videoId) {
                                Image(systemName: "arrow.down.circle.fill").font(.system(size: 12))
                            }
                            Text(subtitle).lineLimit(1)
                        }
                        .font(.system(size: 14))
                        .foregroundStyle(YT.secondary)
                    }
                    Spacer(minLength: 4)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Menu { TrackMenuItems(track: track) } label: {
                Image(systemName: "ellipsis")
                    .rotationEffect(.degrees(90))
                    .foregroundStyle(YT.secondary)
                    .frame(width: 32, height: 44)
            }
        }
        // Offline, what can't play is dimmed.
        .opacity(!Connectivity.shared.online && !library.isDownloaded(track.videoId) ? 0.4 : 1)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .trackMenu(track)
    }

    private var subtitle: String {
        guard let seconds = track.durationSecs else { return track.artist }
        return "\(track.artist) • \(formatTime(Double(seconds)))"
    }

    private var isCurrent: Bool { player.current?.videoId == track.videoId }
}

/// An album, artist or playlist in a list.
struct ItemRow: View {
    let item: Item

    var body: some View {
        NavigationLink(value: Route.page(item.browseId)) {
            HStack(spacing: 14) {
                Artwork(source: item.artwork, size: 56, round: item.kind == .artist)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text(item.title).font(.system(size: 16)).lineLimit(1)
                        if item.explicit { ExplicitBadge() }
                    }
                    Text(item.subtitle)
                        .font(.system(size: 14))
                        .foregroundStyle(YT.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(PrefetchOnPress(item: item))
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .onAppear { PageCache.shared.note(item) }
    }
}

/// A card in a shelf: square art (round for artists, 16:9 for videos),
/// title and subtitle under it.
struct CardView: View {
    let item: Item
    /// The shelf's songs, played as a list from this one.
    var songs: [Track] = []
    var wide = false
    @Environment(Player.self) private var player

    private var width: CGFloat { wide ? 280 : 160 }
    private var round: Bool { item.kind == .artist }

    var body: some View {
        let content = VStack(alignment: round ? .center : .leading, spacing: 6) {
            Group {
                if wide {
                    WideArtwork(source: item.artwork, width: width)
                } else {
                    Artwork(source: item.artwork, size: width, round: round)
                }
            }
            .overlay(alignment: .bottomLeading) {
                if item.explicit { ExplicitBadge().padding(6) }
            }
            Text(item.title)
                .font(.system(size: 15, weight: .medium))
                .lineLimit(2)
                .multilineTextAlignment(round ? .center : .leading)
            Text(item.subtitle)
                .font(.system(size: 13))
                .foregroundStyle(YT.secondary)
                .lineLimit(2)
                .multilineTextAlignment(round ? .center : .leading)
        }
        .frame(width: width, alignment: round ? .top : .topLeading)
        .contentShape(Rectangle())

        if let track = item.track {
            Button {
                player.playFrom(songs.isEmpty ? [track] : songs, at: songs.firstIndex(of: track) ?? 0)
            } label: { content }
                .buttonStyle(.plain)
                .trackMenu(track)
        } else {
            NavigationLink(value: Route.page(item.browseId)) { content }
                .buttonStyle(PrefetchOnPress(item: item))
                .onAppear { PageCache.shared.note(item) }
        }
    }
}

/// A video's 16:9 thumbnail (music video shelves).
struct WideArtwork: View {
    let source: ArtworkSource
    let width: CGFloat

    var body: some View {
        let url: URL? = switch source {
        case let .video(id): URL(string: "https://i.ytimg.com/vi/\(id)/mqdefault.jpg")
        case let .url(url): url
        case .none: nil
        }
        AsyncImage(url: url) { $0.resizable().scaledToFill() } placeholder: { YT.raised }
            .frame(width: width, height: width * 9 / 16)
            .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

/// An Explore category ("Chill", "Workout"): a dark tile with a coloured
/// edge, as YouTube Music draws them.
struct MoodTile: View {
    let item: Item

    var body: some View {
        NavigationLink(value: Route.page(item.browseId)) {
            HStack(spacing: 0) {
                Rectangle()
                    .fill(Color(hue: Double(abs(item.title.hashValue) % 360) / 360, saturation: 0.7, brightness: 0.9))
                    .frame(width: 6)
                Text(item.title)
                    .font(.system(size: 15, weight: .semibold))
                    .lineLimit(1)
                    .padding(.horizontal, 12)
                Spacer(minLength: 0)
            }
            .frame(width: 170, height: 48)
            .background(YT.surface)
            .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(PrefetchOnPress(item: item))
        .onAppear { PageCache.shared.note(item) }
    }
}

/// A shelf: its header (strapline, title, "Play all"), then its cards —
/// in a row, as a three-row grid of tiles (song shelves like "Quick
/// picks"), as wide video cards, or as mood tiles.
struct ShelfView: View {
    let shelf: Shelf
    var showTitle = true
    @Environment(Player.self) private var player

    var body: some View {
        let songs = shelf.items.compactMap(\.track)
        VStack(alignment: .leading, spacing: 12) {
            if showTitle, !shelf.title.isEmpty {
                HStack(spacing: 12) {
                    if let art = shelf.straplineArt {
                        Artwork(source: .url(art), size: 36, round: true)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        if !shelf.strapline.isEmpty {
                            Text(shelf.strapline.uppercased())
                                .font(.system(size: 12, weight: .medium))
                                .foregroundStyle(YT.secondary)
                        }
                        Text(shelf.title)
                            .font(YT.sectionTitle)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    if songs.count > 1 {
                        OutlinePill(title: "Play all") { player.playFrom(songs, at: 0) }
                    }
                }
                .padding(.horizontal)
            }
            if isMoods {
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHGrid(rows: [GridItem(.fixed(48)), GridItem(.fixed(48))], spacing: 10) {
                        ForEach(shelf.items) { MoodTile(item: $0) }
                    }
                    .padding(.horizontal)
                }
            } else if shelf.style == .grid, !songs.isEmpty {
                QuickPicks(songs: songs)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(alignment: .top, spacing: 14) {
                        ForEach(shelf.items) { CardView(item: $0, songs: songs, wide: shelf.style == .wide) }
                    }
                    .padding(.horizontal)
                }
            }
        }
    }

    /// Explore's moods and genres: titles only, no pictures.
    private var isMoods: Bool {
        shelf.items.allSatisfy { $0.track == nil && $0.thumbnail == nil && $0.subtitle.isEmpty }
    }
}

/// "Quick picks": square tiles three rows high with the title over the
/// picture, paged sideways, dots under them.
struct QuickPicks: View {
    let songs: [Track]
    @Environment(Player.self) private var player
    @State private var visible: String?
    @State private var width: CGFloat = 390

    private var tile: CGFloat { max(80, (width - 32 - 12) / 3) }
    private var pages: Int { (songs.count + 8) / 9 }
    private var page: Int {
        guard let visible, let i = songs.firstIndex(where: { $0.videoId == visible }) else { return 0 }
        return min(pages - 1, (i + 4) / 9)
    }

    var body: some View {
        VStack(spacing: 12) {
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHGrid(rows: Array(repeating: GridItem(.fixed(tile), spacing: 6), count: 3), spacing: 6) {
                    ForEach(Array(songs.enumerated()), id: \.element.videoId) { i, track in
                        tileView(track, index: i)
                    }
                }
                .scrollTargetLayout()
            }
            .contentMargins(.horizontal, 16, for: .scrollContent)
            .scrollTargetBehavior(.viewAligned)
            .scrollPosition(id: $visible)
            .frame(height: tile * 3 + 12)
            if pages > 1 {
                HStack(spacing: 6) {
                    ForEach(0..<pages, id: \.self) { p in
                        Circle()
                            .fill(.white.opacity(p == page ? 1 : 0.35))
                            .frame(width: 7, height: 7)
                    }
                }
                .frame(maxWidth: .infinity)
            }
        }
        .background(GeometryReader { geo in
            Color.clear.onAppear { width = geo.size.width }.onChange(of: geo.size.width) { width = geo.size.width }
        })
    }

    private func tileView(_ track: Track, index: Int) -> some View {
        Button { player.playFrom(songs, at: index) } label: {
            ZStack(alignment: .bottomLeading) {
                Artwork(source: .video(track.videoId), size: tile)
                LinearGradient(colors: [.clear, .black.opacity(0.75)], startPoint: .center, endPoint: .bottom)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                Text(track.title)
                    .font(.system(size: 14, weight: .semibold))
                    .lineLimit(1)
                    .padding(.horizontal, 8)
                    .padding(.bottom, 7)
            }
            .frame(width: tile, height: tile)
            .overlay {
                if player.current?.videoId == track.videoId {
                    RoundedRectangle(cornerRadius: 5).stroke(.white, lineWidth: 3)
                }
            }
        }
        .buttonStyle(.plain)
        .trackMenu(track)
    }
}

/// Above the tab bar: the song, AirPlay, play / pause; a tap opens the player.
struct MiniPlayer: View {
    @Environment(Player.self) private var player

    var body: some View {
        if let track = player.current {
            VStack(spacing: 0) {
                ProgressLine()
                HStack(spacing: 12) {
                    Artwork(source: .video(track.videoId), size: 44)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(track.title)
                            .font(.system(size: 15, weight: .semibold))
                            .lineLimit(1)
                        if let status = player.statusText {
                            Text(status)
                                .font(.system(size: 13))
                                .foregroundStyle(player.error == nil ? YT.secondary : Color.red)
                                .lineLimit(1)
                        } else {
                            HStack(spacing: 5) {
                                if track.explicit { ExplicitBadge() }
                                Text(track.artist).lineLimit(1)
                            }
                            .font(.system(size: 13))
                            .foregroundStyle(YT.secondary)
                        }
                    }
                    Spacer(minLength: 4)
                    #if os(iOS)
                    AirPlayButton().frame(width: 40, height: 40)
                    #endif
                    Button { player.togglePlay() } label: {
                        Group {
                            if player.isBuffering, player.isPlaying {
                                ProgressView()
                            } else {
                                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                                    .font(.system(size: 24))
                                    .contentTransition(.symbolEffect(.replace))
                            }
                        }
                        .frame(width: 44, height: 44)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.leading, 10)
                .padding(.trailing, 8)
                .padding(.vertical, 6)
            }
            .background(YT.surface)
            .contentShape(Rectangle())
            .onTapGesture { player.showNowPlaying = true }
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }
}

/// Its own view so the twice-a-second position only redraws this line.
private struct ProgressLine: View {
    @Environment(Player.self) private var player

    var body: some View {
        GeometryReader { geo in
            Rectangle()
                .fill(.white)
                .frame(width: geo.size.width * progress)
        }
        .frame(height: 2)
        .background(.white.opacity(0.15))
    }

    private var progress: CGFloat {
        player.duration > 0 ? min(1, player.position / player.duration) : 0
    }
}
