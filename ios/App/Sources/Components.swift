import SwiftUI
import YTMKit

struct RootView: View {
    @Environment(Player.self) private var player
    @Environment(Router.self) private var router

    var body: some View {
        @Bindable var router = router
        @Bindable var player = player
        TabView(selection: $router.tab) {
            NavigationStack(path: $router.home) { HomeView().routes() }
                .miniPlayer()
                .tabItem { Label("Home", systemImage: "house.fill") }
                .tag(AppTab.home)
            NavigationStack(path: $router.search) { SearchView().routes() }
                .miniPlayer()
                .tabItem { Label("Search", systemImage: "magnifyingglass") }
                .tag(AppTab.search)
            NavigationStack(path: $router.library) { LibraryView().routes() }
                .miniPlayer()
                .tabItem { Label("Library", systemImage: "square.stack.fill") }
                .tag(AppTab.library)
        }
        .sheet(isPresented: $player.showNowPlaying) {
            NowPlayingView()
                .presentationDragIndicator(.visible)
        }
    }
}

extension View {
    func routes() -> some View {
        navigationDestination(for: Route.self) { route in
            switch route {
            case let .page(id): PageView(id: id)
            }
        }
    }

    func miniPlayer() -> some View {
        safeAreaInset(edge: .bottom, spacing: 0) { MiniPlayer() }
    }

    func trackMenu(_ track: Track) -> some View {
        modifier(TrackMenu(track: track))
    }
}

/// Long-press actions for a song.
struct TrackMenu: ViewModifier {
    let track: Track
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router

    func body(content: Content) -> some View {
        content.contextMenu {
            Button("Play next", systemImage: "text.insert") { player.playNext(track) }
            Button("Add to queue", systemImage: "text.append") { player.addToQueue(track) }
            Button("Start radio", systemImage: "dot.radiowaves.left.and.right") { player.startRadio(track) }
            Divider()
            let liked = library.isLiked(track)
            Button(liked ? "Remove from liked" : "Like", systemImage: liked ? "heart.slash" : "heart") {
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
        }
    }
}

/// YouTube Music's "E": explicit lyrics (18+).
struct ExplicitBadge: View {
    var body: some View {
        Text("E")
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(.secondary)
            .frame(width: 15, height: 15)
            .background(.white.opacity(0.15), in: RoundedRectangle(cornerRadius: 3))
            .accessibilityLabel("Explicit")
    }
}

struct TrackRow: View {
    let track: Track
    let list: [Track]
    let index: Int
    /// Album pages number their songs instead of showing art.
    var number: Int?
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library

    var body: some View {
        Button { player.play(list, at: index) } label: {
            HStack(spacing: 12) {
                if let number {
                    Text("\(number)")
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 28)
                } else {
                    Artwork(source: .video(track.videoId), size: 48)
                }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(track.title)
                            .lineLimit(1)
                            .foregroundStyle(isCurrent ? Color.accentColor : Color.primary)
                        if track.explicit { ExplicitBadge() }
                    }
                    Text(track.artist)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if library.isDownloaded(track.videoId) {
                    Image(systemName: "arrow.down.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let seconds = track.durationSecs {
                    Text(formatTime(Double(seconds)))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .trackMenu(track)
    }

    private var isCurrent: Bool { player.current?.videoId == track.videoId }
}

/// An album, artist or playlist in a list.
struct ItemRow: View {
    let item: Item

    var body: some View {
        NavigationLink(value: Route.page(item.browseId)) {
            HStack(spacing: 12) {
                Artwork(source: item.artwork, size: 56, round: item.kind == .artist)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(item.title).lineLimit(1)
                        if item.explicit { ExplicitBadge() }
                    }
                    Text(item.subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
    }
}

struct CardView: View {
    let item: Item
    /// The shelf's songs, played as a list from this one.
    var songs: [Track] = []
    @Environment(Player.self) private var player

    var body: some View {
        let content = VStack(alignment: .leading, spacing: 6) {
            Artwork(source: item.artwork, size: 150, round: item.kind == .artist)
                .overlay(alignment: .bottomLeading) {
                    if item.explicit { ExplicitBadge().padding(6) }
                }
            Text(item.title)
                .font(.subheadline.weight(.medium))
                .lineLimit(2)
            Text(item.subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
        }
        .frame(width: 150, alignment: .topLeading)
        .contentShape(Rectangle())

        if let track = item.track {
            Button {
                player.play(songs.isEmpty ? [track] : songs, at: songs.firstIndex(of: track) ?? 0)
            } label: { content }
                .buttonStyle(.plain)
                .trackMenu(track)
        } else {
            NavigationLink(value: Route.page(item.browseId)) { content }
                .buttonStyle(.plain)
        }
    }
}

struct ShelfView: View {
    let shelf: Shelf
    var showTitle = true

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if showTitle {
                Text(shelf.title)
                    .font(.title3.bold())
                    .padding(.horizontal)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: 14) {
                    let songs = shelf.items.compactMap(\.track)
                    ForEach(shelf.items) { CardView(item: $0, songs: songs) }
                }
                .padding(.horizontal)
            }
        }
    }
}

struct MiniPlayer: View {
    @Environment(Player.self) private var player

    var body: some View {
        if let track = player.current {
            HStack(spacing: 12) {
                Artwork(source: .video(track.videoId), size: 42)
                VStack(alignment: .leading, spacing: 2) {
                    Text(track.title)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    Text(player.error ?? track.artist)
                        .font(.caption)
                        .foregroundStyle(player.error == nil ? Color.secondary : Color.red)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Button { player.togglePlay() } label: {
                    if player.isBuffering {
                        ProgressView()
                    } else {
                        Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                            .font(.title2)
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
                .frame(width: 44, height: 44)
                Button { player.next() } label: {
                    Image(systemName: "forward.fill")
                        .font(.title3)
                        .frame(width: 40, height: 44)
                }
            }
            .buttonStyle(.plain)
            .padding(.leading, 8)
            .padding(.trailing, 4)
            .padding(.vertical, 6)
            .background(.regularMaterial)
            .overlay(alignment: .bottom) { ProgressLine() }
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .contentShape(Rectangle())
            .onTapGesture { player.showNowPlaying = true }
            .padding(.horizontal, 8)
            .padding(.bottom, 4)
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
                .fill(Color.accentColor)
                .frame(width: geo.size.width * progress)
        }
        .frame(height: 2)
    }

    private var progress: CGFloat {
        player.duration > 0 ? min(1, player.position / player.duration) : 0
    }
}
