import AVKit
import SwiftUI
import YTMKit

/// YouTube Music's player: ⌄, the song / video switch and ⋮ at the top, the
/// big cover (or the clip), the title with its ›, the artist, a row of pills
/// (👍|👎, Lyrics, Save, Share, Download, Mix), a thin scrubber, the
/// controls, and "Up next" at the bottom: a sheet with the queue, the lyrics
/// and related music — over the cover's own colours.
struct NowPlayingView: View {
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router
    @State private var sheet: Pane?
    /// The clip instead of the cover; where it started and where it got.
    @State private var video = false
    @State private var videoStart: TimeInterval = 0
    @State private var videoAt: TimeInterval?
    @State private var addingToPlaylist = false
    /// How far the cover is dragged sideways (swipe to the next / previous song).
    @State private var coverOffset: CGFloat = 0

    enum Pane: CaseIterable, Identifiable {
        case queue, lyrics, related

        var id: Self { self }

        var title: LocalizedStringKey {
            switch self {
            case .queue: "Up next"
            case .lyrics: "Lyrics"
            case .related: "Related"
            }
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            if let track = player.current {
                topBar(track)
                Group {
                    if video && !Connectivity.shared.online {
                        ContentUnavailableView("You're offline", systemImage: "wifi.slash",
                                               description: Text("The video needs the internet. The song itself plays if it's downloaded."))
                    } else if video {
                        VideoPane(videoId: track.videoId, start: videoStart) { videoAt = $0 }
                            .id(track.videoId)
                    } else {
                        cover(track)
                    }
                }
                .frame(maxHeight: .infinity)
                header(track)
                actions(track)
                Scrubber()
                controls
                upNext
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
        .sheet(item: $sheet) { start in
            if let track = player.current { PanesSheet(track: track, tab: start) }
        }
    }

    /// The grab handle and "Up next", which open the sheet.
    private var upNext: some View {
        Button { sheet = .queue } label: {
            VStack(spacing: 8) {
                Capsule().fill(.white.opacity(0.3)).frame(width: 36, height: 4)
                Text("Up next")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(YT.secondary)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .gesture(DragGesture(minimumDistance: 20).onEnded { if $0.translation.height < 0 { sheet = .queue } })
    }

    private func topBar(_ track: Track) -> some View {
        HStack {
            Button { player.showNowPlaying = false } label: {
                Image(systemName: "chevron.down").font(.system(size: 20, weight: .semibold)).frame(width: 40, height: 40)
            }
            .buttonStyle(.plain)
            Spacer()
            MediaSwitch(video: $video)
            Spacer()
            #if os(iOS)
            AirPlayButton().frame(width: 40, height: 40)
            #endif
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
                .offset(x: coverOffset)
                .opacity(1 - min(0.6, abs(coverOffset) / max(geo.size.width, 1) * 0.6))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                // The cover itself plays and pauses.
                .contentShape(Rectangle())
                .onTapGesture { player.togglePlay() }
                // YouTube Music's swipe: the cover follows the finger; far
                // enough and the next (left) or previous (right) song slides
                // in. Vertical drags still close the player (the sheet's).
                .simultaneousGesture(
                    DragGesture(minimumDistance: 15)
                        .onChanged { drag in
                            guard abs(drag.translation.width) > abs(drag.translation.height) else { return }
                            coverOffset = drag.translation.width
                        }
                        .onEnded { drag in
                            let width = geo.size.width
                            let dx = drag.translation.width
                            guard abs(dx) > abs(drag.translation.height), abs(dx) > width * 0.25 else {
                                withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) { coverOffset = 0 }
                                return
                            }
                            let forward = dx < 0
                            withAnimation(.easeIn(duration: 0.14)) { coverOffset = forward ? -width : width }
                            DispatchQueue.main.asyncAfter(deadline: .now() + 0.14) {
                                if forward { player.next() } else { player.previous(always: true) }
                                coverOffset = forward ? width : -width
                                withAnimation(.easeOut(duration: 0.22)) { coverOffset = 0 }
                            }
                        }
                )
                .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
                .accessibilityAction(named: "Next song") { player.next() }
                .accessibilityAction(named: "Previous song") { player.previous(always: true) }
        }
        .transition(.opacity)
    }

    private func header(_ track: Track) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            // The title and its ›: the artist's page.
            Button {
                player.showNowPlaying = false
                router.openArtist(of: track)
            } label: {
                HStack(spacing: 6) {
                    Text(track.title)
                        .font(.system(size: 24, weight: .bold))
                        .lineLimit(1)
                    if track.explicit { ExplicitBadge() }
                    Image(systemName: "chevron.right").font(.system(size: 18, weight: .bold))
                }
            }
            .buttonStyle(.plain)
            Text(track.artist)
                .font(.system(size: 17))
                .foregroundStyle(YT.secondary)
                .lineLimit(1)
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
                .frame(height: 38)
                .background(YT.pill, in: Capsule())
                pill("Lyrics", icon: "quote.opening") { sheet = .lyrics }
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
                        .frame(height: 38)
                        .background(YT.pill, in: Capsule())
                }
                pill("Mix", icon: "dot.radiowaves.left.and.right") { player.startRadio(track) }
            }
            .buttonStyle(.plain)
            .font(.system(size: 14, weight: .medium))
        }
    }

    private func pill(_ title: LocalizedStringKey, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon)
                .padding(.horizontal, 14)
                .frame(height: 38)
                .background(YT.pill, in: Capsule())
        }
    }

    private var controls: some View {
        HStack {
            Button { player.shuffleUpcoming() } label: {
                Image(systemName: "shuffle").font(.system(size: 20))
            }
            Spacer()
            Button { player.previous() } label: {
                Image(systemName: "backward.end.fill").font(.system(size: 34))
            }
            Spacer()
            Button { player.togglePlay() } label: {
                ZStack {
                    Circle().fill(.white).frame(width: 80, height: 80)
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
                Image(systemName: "forward.end.fill").font(.system(size: 34))
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
struct PlaylistPicker: View {
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

/// A thin line like YouTube Music's (its own view so ticks don't redraw
/// the whole screen); the times show while it's dragged.
private struct Scrubber: View {
    @Environment(Player.self) private var player
    @State private var dragging: Double?

    var body: some View {
        let duration = max(player.duration, 1)
        let shown = dragging ?? player.position
        VStack(spacing: 4) {
            GeometryReader { geo in
                let fraction = min(max(shown / duration, 0), 1)
                let thick: CGFloat = dragging == nil ? 3 : 5
                ZStack(alignment: .leading) {
                    Capsule().fill(.white.opacity(0.25)).frame(height: thick)
                    Capsule().fill(.white).frame(width: geo.size.width * fraction, height: thick)
                    if dragging != nil {
                        Circle().fill(.white).frame(width: 14, height: 14)
                            .offset(x: geo.size.width * fraction - 7)
                    }
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { dragging = duration * min(max($0.location.x / geo.size.width, 0), 1) }
                    .onEnded { _ in
                        if let target = dragging { player.seek(to: target) }
                        dragging = nil
                    })
            }
            .frame(height: 24)
            .animation(.easeOut(duration: 0.15), value: dragging == nil)
            if dragging != nil {
                HStack {
                    Text(formatTime(shown))
                    Spacer()
                    Text(formatTime(player.duration))
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(YT.secondary)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Position")
        .accessibilityValue(formatTime(shown))
        .accessibilityAdjustableAction { direction in
            player.seek(to: player.position + (direction == .increment ? 10 : -10))
        }
    }
}

/// "Up next" / "Lyrics" / "Related": the sheet under the player.
private struct PanesSheet: View {
    let track: Track
    @State var tab: NowPlayingView.Pane
    @Environment(Player.self) private var player

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ForEach(NowPlayingView.Pane.allCases) { target in
                    Button { withAnimation { tab = target } } label: {
                        VStack(spacing: 8) {
                            Text(target.title)
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(tab == target ? .white : YT.secondary)
                            Rectangle().fill(tab == target ? Color.white : .clear).frame(width: 48, height: 2)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 18)
                    }
                    .buttonStyle(.plain)
                }
            }
            Divider().overlay(.white.opacity(0.1))
            Group {
                switch tab {
                case .queue: QueuePane()
                case .lyrics: LyricsPane(track: player.current ?? track)
                case .related: RelatedPane(track: player.current ?? track)
                }
            }
            .frame(maxHeight: .infinity)
            .padding(.horizontal, 16)
        }
        .background(YT.surface)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .presentationBackground(YT.surface)
    }
}

/// Karaoke lyrics: big bold lines, the sung one lighting up word by word,
/// instrumental breaks as three dots, a tap seeks to a line.
private struct LyricsPane: View {
    let track: Track
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
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
            // Kept on the phone (a download's, or played lately): offline too.
            lyrics = try await KeptLyrics.find(track, duration: duration, keep: library.downloadedIds)
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
                            // Touch and hold, then drag to reorder; swipe left to remove.
                            Image(systemName: "line.3.horizontal")
                                .foregroundStyle(YT.secondary)
                                .accessibilityHidden(true)
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
