import AVKit
import SwiftUI
import YTMKit

/// YouTube Music's player: ⌄ and the song / video switch at the top, the
/// big cover (or the clip), title and artist, a row of pill actions, the
/// scrubber, the controls, and UP NEXT / LYRICS / RELATED at the bottom —
/// over the cover's own colours.
struct NowPlayingView: View {
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router
    @State private var pane: Pane?
    /// The clip instead of the cover; where it started and where it got.
    @State private var video = false
    @State private var videoStart: TimeInterval = 0
    @State private var videoAt: TimeInterval?
    @State private var addingToPlaylist = false

    enum Pane: CaseIterable {
        case queue, lyrics, related

        var title: LocalizedStringKey {
            switch self {
            case .queue: "UP NEXT"
            case .lyrics: "LYRICS"
            case .related: "RELATED"
            }
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            if let track = player.current {
                topBar(track)
                Group {
                    switch pane {
                    case nil where video && !Connectivity.shared.online:
                        ContentUnavailableView("You're offline", systemImage: "wifi.slash",
                                               description: Text("The video needs the internet. The song itself plays if it's downloaded."))
                    case nil where video:
                        VideoPane(videoId: track.videoId, start: videoStart) { videoAt = $0 }
                            .id(track.videoId)
                    case nil: cover(track)
                    case .lyrics: LyricsPane(track: track)
                    case .queue: QueuePane()
                    case .related: RelatedPane(track: track)
                    }
                }
                .frame(maxHeight: .infinity)
                .animation(.easeInOut(duration: 0.25), value: pane)
                header(track)
                actions(track)
                Scrubber()
                controls
                tabs
            } else {
                ContentUnavailableView("Nothing playing", systemImage: "music.note")
            }
        }
        .padding(.horizontal, 22)
        .padding(.top, 12)
        .padding(.bottom, 4)
        .onChange(of: video) { _, on in
            if on {
                videoAt = nil
                videoStart = player.pauseForVideo()
            } else {
                player.resumeAfterVideo(at: videoAt)
            }
        }
        .onChange(of: player.current?.videoId) { _, _ in
            // Another song: back to its audio, from its start.
            if video {
                videoAt = 0
                video = false
            }
        }
        .onDisappear {
            if video { video = false; player.resumeAfterVideo(at: videoAt) }
        }
        .background {
            ZStack {
                YT.background
                if let track = player.current {
                    Artwork(source: .video(track.videoId), size: 200)
                        .scaleEffect(4)
                        .blur(radius: 70)
                        .opacity(0.65)
                }
                LinearGradient(colors: [.black.opacity(0.15), .black.opacity(0.8)], startPoint: .top, endPoint: .bottom)
            }
            .ignoresSafeArea()
        }
        .sheet(isPresented: $addingToPlaylist) {
            if let track = player.current { PlaylistPicker(track: track) }
        }
    }

    private func topBar(_ track: Track) -> some View {
        HStack {
            Button { player.showNowPlaying = false } label: {
                Image(systemName: "chevron.down").font(.system(size: 20, weight: .semibold)).frame(width: 40, height: 40)
            }
            .buttonStyle(.plain)
            Spacer()
            if pane == nil { MediaSwitch(video: $video) }
            Spacer()
            Menu {
                sleepMenu
                TrackMenuItems(track: track)
            } label: {
                Image(systemName: "ellipsis")
                    .rotationEffect(.degrees(90))
                    .font(.system(size: 18, weight: .semibold))
                    .frame(width: 40, height: 40)
            }
        }
    }

    private func cover(_ track: Track) -> some View {
        GeometryReader { geo in
            Artwork(source: .video(track.videoId), size: min(geo.size.width, geo.size.height))
                .shadow(color: .black.opacity(0.5), radius: 24, y: 12)
                .scaleEffect(player.isPlaying ? 1 : 0.9)
                .animation(.spring(response: 0.45, dampingFraction: 0.7), value: player.isPlaying)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                // The cover itself plays and pauses.
                .contentShape(Rectangle())
                .onTapGesture { player.togglePlay() }
                .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
        }
        .transition(.opacity)
    }

    private func header(_ track: Track) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(track.title)
                    .font(.system(size: 22, weight: .bold))
                    .lineLimit(1)
                if track.explicit { ExplicitBadge() }
            }
            Button {
                player.showNowPlaying = false
                router.openArtist(of: track)
            } label: {
                Text(track.artist)
                    .font(.system(size: 16))
                    .foregroundStyle(YT.secondary)
                    .lineLimit(1)
            }
            .buttonStyle(.plain)
            if let status = player.statusText {
                Text(status)
                    .font(.caption)
                    .foregroundStyle(player.error == nil ? YT.secondary : Color.red)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 👍 | 👎, Save, Download, Share — YouTube Music's pill row.
    private func actions(_ track: Track) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                let liked = library.isLiked(track)
                HStack(spacing: 0) {
                    Button { library.toggleLike(track) } label: {
                        Image(systemName: liked ? "hand.thumbsup.fill" : "hand.thumbsup")
                            .symbolEffect(.bounce, value: liked)
                            .padding(.leading, 14)
                            .padding(.trailing, 12)
                    }
                    Rectangle().fill(.white.opacity(0.25)).frame(width: 1, height: 18)
                    Button { player.dislike(track) } label: {
                        Image(systemName: "hand.thumbsdown")
                            .padding(.leading, 12)
                            .padding(.trailing, 14)
                    }
                }
                .frame(height: 34)
                .background(YT.raised, in: Capsule())
                pill("Save", icon: "text.badge.plus") { addingToPlaylist = true }
                if library.isDownloaded(track.videoId) {
                    pill("Downloaded", icon: "arrow.down.circle.fill") { library.removeDownload(track) }
                } else if library.downloading.contains(track.videoId) {
                    pill("Downloading…", icon: "arrow.down.circle") {}
                } else {
                    pill("Download", icon: "arrow.down.circle") { library.download(track) }
                }
                ShareLink(item: URL(string: "https://music.youtube.com/watch?v=\(track.videoId)")!) {
                    Label("Share", systemImage: "square.and.arrow.up")
                        .font(.system(size: 14, weight: .medium))
                        .padding(.horizontal, 14)
                        .frame(height: 34)
                        .background(YT.raised, in: Capsule())
                }
            }
            .buttonStyle(.plain)
            .font(.system(size: 14, weight: .medium))
        }
    }

    private func pill(_ title: LocalizedStringKey, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon)
                .padding(.horizontal, 14)
                .frame(height: 34)
                .background(YT.raised, in: Capsule())
        }
    }

    private var controls: some View {
        HStack {
            Button { player.shuffleUpcoming() } label: {
                Image(systemName: "shuffle").font(.system(size: 20))
            }
            Spacer()
            Button { player.previous() } label: {
                Image(systemName: "backward.end.fill").font(.system(size: 30))
            }
            Spacer()
            Button { player.togglePlay() } label: {
                ZStack {
                    Circle().fill(.white).frame(width: 72, height: 72)
                    if player.isBuffering, player.isPlaying {
                        ProgressView().tint(.black)
                    } else {
                        Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                            .font(.system(size: 30))
                            .foregroundStyle(.black)
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
            }
            Spacer()
            Button { player.next() } label: {
                Image(systemName: "forward.end.fill").font(.system(size: 30))
            }
            Spacer()
            Button { player.repeatMode = player.repeatMode.next } label: {
                Image(systemName: player.repeatMode == .one ? "repeat.1" : "repeat")
                    .font(.system(size: 20))
                    .foregroundStyle(player.repeatMode == .off ? YT.secondary : .white)
            }
        }
        .buttonStyle(.plain)
    }

    /// UP NEXT / LYRICS / RELATED, and AirPlay.
    private var tabs: some View {
        HStack {
            ForEach(Pane.allCases, id: \.self) { target in
                Button {
                    withAnimation { pane = pane == target ? nil : target }
                } label: {
                    Text(target.title)
                        .font(.system(size: 13, weight: .bold))
                        .tracking(0.6)
                        .foregroundStyle(pane == target ? .white : YT.secondary)
                        .frame(maxWidth: .infinity, minHeight: 40)
                }
                .buttonStyle(.plain)
            }
            #if os(iOS)
            AirPlayButton().frame(width: 40, height: 40)
            #endif
        }
    }

    /// Sleep timer: pause after a while, or when the song ends.
    @ViewBuilder private var sleepMenu: some View {
        let on = player.sleepUntil != nil || player.sleepAtEndOfSong
        Menu(on ? "Sleep timer (on)" : "Sleep timer", systemImage: on ? "moon.zzz.fill" : "moon.zzz") {
            ForEach([15, 30, 60], id: \.self) { minutes in
                Button("\(minutes) min") { player.setSleep(minutes: minutes) }
            }
            Button("End of song") { player.setSleep(minutes: nil) }
            if on {
                Button("Turn off", role: .destructive) { player.cancelSleep() }
            }
        }
        Divider()
    }
}

/// "Save": the song into one of the playlists, or a new one.
private struct PlaylistPicker: View {
    let track: Track
    @Environment(Library.self) private var library
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Button("New playlist", systemImage: "plus") {
                    library.createPlaylist("", with: track)
                    dismiss()
                }
                ForEach(library.playlists) { playlist in
                    Button {
                        library.add(track, to: playlist.id)
                        dismiss()
                    } label: {
                        HStack(spacing: 12) {
                            Artwork(source: playlist.tracks.first.map { .video($0.videoId) } ?? .none, size: 44)
                            Text(playlist.name)
                        }
                    }
                }
            }
            .navigationTitle("Save to playlist")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// Position slider; its own view so ticks don't redraw the whole screen.
private struct Scrubber: View {
    @Environment(Player.self) private var player
    @State private var dragging: Double?

    var body: some View {
        let shown = dragging ?? player.position
        VStack(spacing: 2) {
            Slider(value: Binding(get: { shown }, set: { dragging = $0 }),
                   in: 0...max(player.duration, 1),
                   onEditingChanged: { editing in
                       if !editing, let target = dragging {
                           player.seek(to: target)
                           dragging = nil
                       }
                   })
            HStack {
                Text(formatTime(shown))
                Spacer()
                Text("-" + formatTime(max(0, player.duration - shown)))
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
        }
    }
}

/// Karaoke lyrics: big bold lines, the sung one lighting up word by word,
/// instrumental breaks as three dots, a tap seeks to a line.
private struct LyricsPane: View {
    let track: Track
    @Environment(Player.self) private var player
    @State private var lyrics: Lyrics?
    @State private var loading = true
    @State private var failure: Error?

    var body: some View {
        Group {
            if let lyrics {
                if lyrics.synced {
                    // Redraws only while on screen; 10 per second is enough
                    // for words to light up on time.
                    TimelineView(.periodic(from: .now, by: 0.1)) { _ in
                        SyncedLyrics(lyrics: lyrics, progress: lyrics.progress(at: player.exactPosition + 0.15))
                    }
                } else {
                    ScrollView(showsIndicators: false) {
                        Text(lyrics.lines.map(\.text).joined(separator: "\n"))
                            .font(.title3.weight(.medium))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, 24)
                    }
                }
            } else if loading {
                LoadingView()
            } else if let failure {
                ErrorView(error: failure) { Task { await load() } }
            } else {
                ContentUnavailableView("No lyrics", systemImage: "quote.bubble")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: track.videoId) { await load() }
        .onChange(of: Connectivity.shared.cameBack) {
            if failure != nil { Task { await load() } }
        }
    }

    private func load() async {
        lyrics = nil
        failure = nil
        loading = true
        let duration = track.durationSecs.map(Double.init) ?? (player.duration > 0 ? player.duration : nil)
        do {
            lyrics = try await Services.lyrics.find(track, duration: duration)
        } catch {
            failure = error
        }
        loading = false
    }
}

private struct SyncedLyrics: View {
    let lyrics: Lyrics
    let progress: LyricsProgress?
    @Environment(Player.self) private var player

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 22) {
                    if lyrics.hasIntro {
                        Dots(lit: progress?.line == -1 ? progress?.words ?? 0 : 0, active: progress?.line == -1)
                            .id(-1)
                    }
                    ForEach(Array(lyrics.lines.enumerated()), id: \.offset) { i, line in
                        lineView(i, line)
                            .id(i)
                            .contentShape(Rectangle())
                            .onTapGesture { if let at = line.at { player.seek(to: at) } }
                    }
                    Text(lyrics.source)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.top, 24)
                .padding(.bottom, 280)
            }
            .onChange(of: progress?.line) { _, line in
                guard let line else { return }
                withAnimation(.spring(response: 0.55, dampingFraction: 0.9)) {
                    proxy.scrollTo(line, anchor: UnitPoint(x: 0, y: 0.3))
                }
            }
            .onAppear {
                if let line = progress?.line { proxy.scrollTo(line, anchor: UnitPoint(x: 0, y: 0.3)) }
            }
        }
        .mask(LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.06),
                                     .init(color: .black, location: 0.9), .init(color: .clear, location: 1)],
                             startPoint: .top, endPoint: .bottom))
    }

    @ViewBuilder
    private func lineView(_ i: Int, _ line: LyricLine) -> some View {
        let current = progress?.line == i
        let sung = progress.map { i < $0.line } ?? false
        Group {
            if line.instrumental {
                Dots(lit: current ? progress?.words ?? 0 : 0, active: current)
            } else if current {
                // Sung words bright, the rest dim; Text concatenation wraps.
                let lit = min(progress?.chars ?? 0, line.text.count)
                (Text(line.text.prefix(lit)).foregroundColor(.white)
                    + Text(line.text.dropFirst(lit)).foregroundColor(.white.opacity(0.38)))
            } else {
                Text(line.text)
                    .foregroundColor(.white.opacity(sung ? 0.22 : 0.38))
            }
        }
        .font(.system(size: 28, weight: .bold))
        .frame(maxWidth: .infinity, alignment: .leading)
        .scaleEffect(current ? 1 : 0.97, anchor: .leading)
        .animation(.spring(response: 0.4, dampingFraction: 0.8), value: current)
    }
}

/// An instrumental break: three dots lighting up until the next line.
private struct Dots: View {
    let lit: Int
    let active: Bool

    var body: some View {
        HStack(spacing: 10) {
            ForEach(0..<3, id: \.self) { d in
                Circle()
                    .fill(.white.opacity(active && d < lit ? 1 : 0.25))
                    .frame(width: active ? 13 : 9, height: active ? 13 : 9)
            }
        }
        .frame(height: 34)
        .animation(.easeOut(duration: 0.45), value: lit)
        .animation(.spring(response: 0.4), value: active)
    }
}

private struct QueuePane: View {
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @State private var naming = false
    @State private var name = ""

    var body: some View {
        List {
            Section {
                ForEach(Array(player.upNext.enumerated()), id: \.offset) { i, track in
                    Button { player.jump(to: player.index + 1 + i) } label: {
                        HStack(spacing: 12) {
                            Artwork(source: .video(track.videoId), size: 40)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(track.title).lineLimit(1)
                                Text(track.artist)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                            Spacer()
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .onDelete { player.removeUpcoming($0) }
                .onMove { player.moveUpcoming($0, $1) }
            } header: {
                HStack {
                    Text("Up next")
                    Spacer()
                    // The whole queue (what played, the song, what's next) as a playlist.
                    Button("Save as playlist") {
                        name = String(localized: "Queue \(Date().formatted(date: .abbreviated, time: .omitted))")
                        naming = true
                    }
                    .textCase(nil)
                }
            }
            .listRowBackground(Color.clear)
        }
        .alert("Save as playlist", isPresented: $naming) {
            TextField("Name", text: $name)
            Button("Save") { library.createPlaylist(name, tracks: player.queue) }
            Button("Cancel", role: .cancel) {}
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .overlay {
            if player.upNext.isEmpty {
                ContentUnavailableView("Queue is empty", systemImage: "list.bullet",
                                       description: Text(player.autoplay ? "Similar songs will follow." : ""))
            }
        }
    }
}

#if os(iOS)
/// AirPlay / Bluetooth output picker.
struct AirPlayButton: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.tintColor = .white
        view.activeTintColor = .white
        view.prioritizesVideoDevices = false
        return view
    }

    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
#endif

/// Songs, albums and artists YouTube Music relates to the playing song.
private struct RelatedPane: View {
    let track: Track
    @State private var shelves: [Shelf]?
    @State private var failure: Error?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 18) {
                ForEach(shelves ?? []) { ShelfView(shelf: $0) }
            }
        }
        .overlay {
            if let failure {
                ErrorView(error: failure) { Task { await load() } }
            } else if shelves == nil {
                LoadingView()
            } else if shelves?.isEmpty == true {
                ContentUnavailableView("Nothing related", systemImage: "square.stack")
            }
        }
        .task(id: track.videoId) { await load() }
        .onChange(of: Connectivity.shared.cameBack) {
            if failure != nil { Task { await load() } }
        }
    }

    private func load() async {
        shelves = nil
        failure = nil
        do {
            shelves = try await Services.catalog.related(track.videoId)
        } catch {
            failure = error
        }
    }
}
