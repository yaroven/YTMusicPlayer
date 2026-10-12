import AVKit
import SwiftUI
import YTMKit

struct NowPlayingView: View {
    @Environment(Player.self) private var player
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router
    @State private var pane: Pane?

    enum Pane {
        case lyrics, queue
    }

    var body: some View {
        VStack(spacing: 18) {
            if let track = player.current {
                Group {
                    switch pane {
                    case nil: cover(track)
                    case .lyrics: LyricsPane(track: track)
                    case .queue: QueuePane()
                    }
                }
                .frame(maxHeight: .infinity)
                .animation(.easeInOut(duration: 0.25), value: pane)
                header(track)
                Scrubber()
                controls
                footer
            } else {
                ContentUnavailableView("Nothing playing", systemImage: "music.note")
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 28)
        .padding(.bottom, 8)
        .background {
            LinearGradient(colors: [Color.accentColor.opacity(0.35), Color.black], startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()
        }
    }

    private func cover(_ track: Track) -> some View {
        GeometryReader { geo in
            Artwork(source: .video(track.videoId), size: min(geo.size.width, geo.size.height))
                .shadow(color: .black.opacity(0.5), radius: 24, y: 12)
                .scaleEffect(player.isPlaying ? 1 : 0.86)
                .animation(.spring(response: 0.45, dampingFraction: 0.7), value: player.isPlaying)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .transition(.opacity)
    }

    private func header(_ track: Track) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(track.title)
                    .font(.title3.bold())
                    .lineLimit(1)
                Button {
                    player.showNowPlaying = false
                    router.openArtist(of: track)
                } label: {
                    Text(track.artist)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                if let error = player.error {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                }
            }
            Spacer()
            let liked = library.isLiked(track)
            Button { library.toggleLike(track) } label: {
                Image(systemName: liked ? "heart.fill" : "heart")
                    .font(.title2)
                    .foregroundStyle(liked ? Color.accentColor : Color.primary)
                    .symbolEffect(.bounce, value: liked)
            }
            .buttonStyle(.plain)
        }
    }

    private var controls: some View {
        HStack {
            Button { player.shuffleUpcoming() } label: {
                Image(systemName: "shuffle").font(.title3)
            }
            Spacer()
            Button { player.previous() } label: {
                Image(systemName: "backward.fill").font(.title)
            }
            Spacer()
            Button { player.togglePlay() } label: {
                ZStack {
                    Circle().fill(.white).frame(width: 72, height: 72)
                    if player.isBuffering {
                        ProgressView().tint(.black)
                    } else {
                        Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                            .font(.title)
                            .foregroundStyle(.black)
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
            }
            Spacer()
            Button { player.next() } label: {
                Image(systemName: "forward.fill").font(.title)
            }
            Spacer()
            Button { player.repeatMode = player.repeatMode.next } label: {
                Image(systemName: player.repeatMode == .one ? "repeat.1" : "repeat")
                    .font(.title3)
                    .foregroundStyle(player.repeatMode == .off ? Color.primary : Color.accentColor)
            }
        }
        .buttonStyle(.plain)
    }

    private var footer: some View {
        HStack {
            toggle(.lyrics, icon: "quote.bubble")
            Spacer()
            #if os(iOS)
            AirPlayButton().frame(width: 44, height: 44)
            #endif
            Spacer()
            toggle(.queue, icon: "list.bullet")
        }
    }

    private func toggle(_ target: Pane, icon: String) -> some View {
        Button {
            pane = pane == target ? nil : target
        } label: {
            Image(systemName: icon)
                .font(.title3)
                .foregroundStyle(pane == target ? Color.accentColor : Color.secondary)
                .frame(width: 44, height: 44)
        }
        .buttonStyle(.plain)
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

private struct LyricsPane: View {
    let track: Track
    @Environment(Player.self) private var player
    @State private var lyrics: Lyrics?
    @State private var loading = true

    var body: some View {
        Group {
            if let lyrics {
                lines(lyrics)
            } else if loading {
                ProgressView()
            } else {
                ContentUnavailableView("No lyrics", systemImage: "quote.bubble")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: track.videoId) {
            lyrics = nil
            loading = true
            let duration = track.durationSecs.map(Double.init) ?? (player.duration > 0 ? player.duration : nil)
            lyrics = try? await Services.lyrics.find(track, duration: duration)
            loading = false
        }
    }

    private func lines(_ lyrics: Lyrics) -> some View {
        let current = lyrics.currentLine(at: player.position + 0.3)
        return ScrollViewReader { proxy in
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach(Array(lyrics.lines.enumerated()), id: \.offset) { i, line in
                        Text(line.text.isEmpty ? "♪" : line.text)
                            .font(lyrics.synced ? .title2.bold() : .body)
                            .foregroundStyle(!lyrics.synced || i == current ? Color.primary : Color.secondary.opacity(0.6))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .id(i)
                            .onTapGesture { if let at = line.at { player.seek(to: at) } }
                    }
                    Text(lyrics.source)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 40)
            }
            .onChange(of: current) { _, line in
                guard let line else { return }
                withAnimation(.easeInOut(duration: 0.4)) { proxy.scrollTo(line, anchor: .center) }
            }
            .onAppear { if let current { proxy.scrollTo(current, anchor: .center) } }
        }
        .mask(LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.08),
                                     .init(color: .black, location: 0.92), .init(color: .clear, location: 1)],
                             startPoint: .top, endPoint: .bottom))
    }
}

private struct QueuePane: View {
    @Environment(Player.self) private var player

    var body: some View {
        List {
            Section("Up next") {
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
            }
            .listRowBackground(Color.clear)
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
private struct AirPlayButton: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.tintColor = .secondaryLabel
        view.activeTintColor = UIColor(Color.accentColor)
        view.prioritizesVideoDevices = false
        return view
    }

    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
#endif
