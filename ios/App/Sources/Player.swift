import AVFoundation
import MediaPlayer
import YTMKit

enum RepeatMode: String, CaseIterable {
    case off, all, one

    var next: RepeatMode {
        switch self {
        case .off: .all
        case .all: .one
        case .one: .off
        }
    }
}

/// Queue + playback: AVQueuePlayer holds the current item and, once it
/// plays, the next one (resolved ahead) for gapless changes. Lock screen /
/// Control Center via MPNowPlayingInfoCenter and MPRemoteCommandCenter;
/// background audio needs `UIBackgroundModes: audio` and the playback
/// audio session category.
@MainActor @Observable
final class Player {
    private(set) var queue: [Track] = []
    private(set) var index = 0
    private(set) var isPlaying = false
    /// Resolving the URL, or AVPlayer waiting for data.
    var isBuffering: Bool { resolving || waiting }
    private var resolving = false
    private var waiting = false
    /// Pause pressed while the song was still resolving.
    @ObservationIgnored private var playWhenReady = true
    private(set) var position: TimeInterval = 0
    private(set) var duration: TimeInterval = 0
    var error: String?
    var showNowPlaying = false
    var repeatMode: RepeatMode = .off {
        didSet {
            av.actionAtItemEnd = repeatMode == .one ? .none : .advance
            queueChanged()
        }
    }
    /// Similar songs after the queue ends (the song's YouTube Music radio).
    var autoplay = UserDefaults.standard.object(forKey: "autoplay") as? Bool ?? true {
        didSet { UserDefaults.standard.set(autoplay, forKey: "autoplay") }
    }
    /// Turn loud masters down to YouTube's loudness target.
    var normalize = UserDefaults.standard.object(forKey: "normalize") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(normalize, forKey: "normalize")
            av.volume = loaded.map(gain) ?? 1
        }
    }

    var current: Track? { queue.indices.contains(index) ? queue[index] : nil }
    var upNext: ArraySlice<Track> { queue.dropFirst(index + 1) }

    private struct Loaded {
        let index: Int
        let videoId: String
        let item: AVPlayerItem
        let loudnessDb: Double?
        let duration: TimeInterval?
    }

    @ObservationIgnored private let library: Library
    @ObservationIgnored private let av = AVQueuePlayer()
    @ObservationIgnored private var loaded: Loaded?
    @ObservationIgnored private var preloaded: Loaded?
    @ObservationIgnored private var loadTask: Task<Void, Never>?
    @ObservationIgnored private var preloadTask: Task<Void, Never>?
    @ObservationIgnored private var observations: [NSKeyValueObservation] = []
    @ObservationIgnored private var itemObservations: [ObjectIdentifier: NSKeyValueObservation] = [:]
    @ObservationIgnored private var timeObserver: Any?
    /// Songs whose URL was already refreshed once after a failure.
    @ObservationIgnored private var retried: Set<String> = []
    @ObservationIgnored private var failuresInARow = 0
    @ObservationIgnored private var radioSeed: String?
    @ObservationIgnored private var artwork: (videoId: String, value: MPMediaItemArtwork)?

    /// `muted`: for the macOS player check (`ios/player-check.sh`).
    init(library: Library, muted: Bool = false) {
        self.library = library
        av.isMuted = muted
        restoreQueue()
        setUpPlayer()
        setUpAudioSession()
        setUpRemoteCommands()
    }

    // MARK: Commands

    func play(_ tracks: [Track], at start: Int = 0) {
        guard tracks.indices.contains(start) else { return }
        queue = tracks
        index = start
        radioSeed = nil
        retried = []
        failuresInARow = 0
        load()
    }

    /// The song, then its radio.
    func startRadio(_ track: Track) {
        play([track])
        appendRadio(of: track)
    }

    func playNext(_ track: Track) {
        guard current != nil else { return play([track]) }
        queue.insert(track, at: index + 1)
        queueChanged()
    }

    func addToQueue(_ track: Track) {
        guard current != nil else { return play([track]) }
        queue.append(track)
        queueChanged()
    }

    func jump(to newIndex: Int) {
        guard queue.indices.contains(newIndex) else { return }
        index = newIndex
        load()
    }

    func removeUpcoming(_ offsets: IndexSet) {
        queue.remove(atOffsets: IndexSet(offsets.map { $0 + index + 1 }))
        queueChanged()
    }

    func moveUpcoming(_ source: IndexSet, _ destination: Int) {
        queue.move(fromOffsets: IndexSet(source.map { $0 + index + 1 }), toOffset: destination + index + 1)
        queueChanged()
    }

    func shuffleUpcoming() {
        guard index + 1 < queue.count else { return }
        queue[(index + 1)...].shuffle()
        queueChanged()
    }

    func togglePlay() {
        if resolving {
            playWhenReady.toggle()
            isPlaying = playWhenReady
            updateNowPlaying()
        } else if isPlaying {
            av.pause()
        } else if loaded == nil {
            load(resumeAt: position)
        } else {
            activateAudioSession()
            av.play()
        }
    }

    func next() {
        guard let n = nextIndex() else { return }
        if let preloaded, preloaded.index == n, av.items().count > 1 {
            av.advanceToNextItem()
        } else {
            index = n
            load()
        }
    }

    func previous() {
        if position > 3 || index == 0 {
            seek(to: 0)
        } else {
            index -= 1
            load()
        }
    }

    func seek(to seconds: TimeInterval) {
        position = seconds
        av.seek(to: CMTime(seconds: seconds, preferredTimescale: 600)) { [weak self] _ in
            Task { @MainActor in self?.updateNowPlaying() }
        }
        updateNowPlaying()
    }

    // MARK: Loading

    private func nextIndex() -> Int? {
        if index + 1 < queue.count { return index + 1 }
        return repeatMode == .all && !queue.isEmpty ? 0 : nil
    }

    /// Resolve and play `current` from scratch.
    private func load(resumeAt: TimeInterval = 0) {
        loadTask?.cancel()
        dropPreload()
        forgetItem(loaded?.item)
        loaded = nil
        // The rate survives removeAllItems: a new item would start by itself.
        av.pause()
        av.removeAllItems()
        guard let track = current else {
            resolving = false
            isPlaying = false
            updateNowPlaying()
            return
        }
        error = nil
        resolving = true
        playWhenReady = true
        isPlaying = true
        position = resumeAt
        duration = Double(track.durationSecs ?? 0)
        saveQueue()
        updateNowPlaying()
        let target = index
        loadTask = Task {
            do {
                let next = try await makeItem(track, index: target)
                guard !Task.isCancelled, index == target else { return }
                loaded = next
                resolving = false
                duration = next.duration ?? duration
                av.volume = gain(next)
                av.insert(next.item, after: nil)
                if resumeAt > 0 {
                    await av.seek(to: CMTime(seconds: resumeAt, preferredTimescale: 600))
                }
                if playWhenReady {
                    activateAudioSession()
                    av.play()
                }
                started(track)
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled, index == target else { return }
                failed(track, error)
            }
        }
    }

    private func makeItem(_ track: Track, index: Int) async throws -> Loaded {
        let stream = try await Services.resolver.stream(track.videoId)
        try Task.checkCancellation()
        let item = AVPlayerItem(url: stream.url)
        if let duration = stream.duration {
            // AVFoundation thinks YouTube's fMP4 is twice as long.
            item.forwardPlaybackEndTime = CMTime(seconds: duration, preferredTimescale: 1000)
        }
        // Enough to ride out a weak signal, not the whole song in memory.
        item.preferredForwardBufferDuration = 30
        watch(item)
        return Loaded(index: index, videoId: track.videoId, item: item, loudnessDb: stream.loudnessDb,
                      duration: stream.duration)
    }

    private func gain(_ loaded: Loaded) -> Float {
        guard normalize, let db = loaded.loudnessDb, db > 0 else { return 1 }
        return Float(pow(10, -db / 20))
    }

    /// A song began: history, the next one, more radio.
    private func started(_ track: Track) {
        failuresInARow = 0
        library.played(track)
        updateNowPlaying()
        preloadNext()
        if autoplay, repeatMode == .off, index >= queue.count - 2, let last = queue.last {
            appendRadio(of: last)
        }
    }

    private func failed(_ track: Track, _ error: Error) {
        resolving = false
        isPlaying = false
        failuresInARow += 1
        self.error = "\(track.title): \(error.localizedDescription)"
        updateNowPlaying()
        // Skip what can't play (age-restricted, removed), but don't run
        // through a whole queue when the network is down.
        guard failuresInARow < 3, let n = nextIndex(), n != index else { return }
        let target = index
        Task {
            try? await Task.sleep(for: .seconds(2))
            guard index == target, loaded == nil else { return }
            index = n
            load()
        }
    }

    private func preloadNext() {
        preloadTask?.cancel()
        guard repeatMode != .one, let n = nextIndex(), loaded != nil else { return }
        let track = queue[n]
        preloadTask = Task {
            guard let next = try? await makeItem(track, index: n), !Task.isCancelled,
                  let loaded, av.items().last === loaded.item else { return }
            preloaded = next
            av.insert(next.item, after: loaded.item)
        }
    }

    private func dropPreload() {
        preloadTask?.cancel()
        if let preloaded {
            av.remove(preloaded.item)
            forgetItem(preloaded.item)
        }
        preloaded = nil
    }

    /// The upcoming order changed: the preloaded song may not be next any more.
    private func queueChanged() {
        if let preloaded, nextIndex() == preloaded.index,
           queue.indices.contains(preloaded.index), queue[preloaded.index].videoId == preloaded.videoId {
            saveQueue()
            return
        }
        dropPreload()
        preloadNext()
        saveQueue()
    }

    private func appendRadio(of track: Track) {
        guard radioSeed != track.videoId else { return }
        radioSeed = track.videoId
        Task {
            guard let radio = try? await Services.catalog.radio(track.videoId) else { return }
            let have = Set(queue.map(\.videoId))
            let more = radio.filter { !have.contains($0.videoId) }
            guard !more.isEmpty else { return }
            queue += more
            if preloaded == nil { preloadNext() }
            saveQueue()
        }
    }

    // MARK: AVPlayer events

    private func setUpPlayer() {
        av.automaticallyWaitsToMinimizeStalling = true
        observations.append(av.observe(\.timeControlStatus, options: [.new]) { [weak self] av, _ in
            let status = av.timeControlStatus
            Task { @MainActor in self?.statusChanged(status) }
        })
        observations.append(av.observe(\.currentItem, options: [.new]) { [weak self] av, _ in
            let item = av.currentItem
            Task { @MainActor in self?.currentItemChanged(item) }
        })
        timeObserver = av.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600),
                                                  queue: .main) { [weak self] time in
            MainActor.assumeIsolated { self?.tick(time) }
        }
        let center = NotificationCenter.default
        center.addObserver(forName: AVPlayerItem.didPlayToEndTimeNotification, object: nil, queue: .main) { [weak self] note in
            let item = note.object as? AVPlayerItem
            MainActor.assumeIsolated { self?.itemEnded(item) }
        }
        center.addObserver(forName: AVPlayerItem.failedToPlayToEndTimeNotification, object: nil, queue: .main) { [weak self] note in
            let item = note.object as? AVPlayerItem
            let error = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
            MainActor.assumeIsolated { self?.itemFailed(item, error) }
        }
    }

    private func watch(_ item: AVPlayerItem) {
        itemObservations[ObjectIdentifier(item)] = item.observe(\.status, options: [.new]) { [weak self] item, _ in
            guard item.status == .failed else { return }
            let error = item.error
            Task { @MainActor in self?.itemFailed(item, error) }
        }
    }

    private func forgetItem(_ item: AVPlayerItem?) {
        if let item { itemObservations[ObjectIdentifier(item)] = nil }
    }

    private func statusChanged(_ status: AVPlayer.TimeControlStatus) {
        let playing = status != .paused
        waiting = status == .waitingToPlayAtSpecifiedRate
        // While resolving, `isPlaying` is the intent (the old item just stopped).
        guard !resolving else { return }
        if playing != isPlaying {
            isPlaying = playing
            updateNowPlaying()
        }
    }

    /// AVQueuePlayer moved on to the preloaded song by itself.
    private func currentItemChanged(_ item: AVPlayerItem?) {
        guard let item, let next = preloaded, next.item === item else { return }
        forgetItem(loaded?.item)
        loaded = next
        preloaded = nil
        index = next.index
        av.volume = gain(next)
        position = 0
        duration = next.duration ?? Double(current?.durationSecs ?? 0)
        retried.remove(next.videoId)
        saveQueue()
        if let track = current { started(track) }
    }

    private func itemEnded(_ item: AVPlayerItem?) {
        guard let item, let loaded, loaded.item === item else { return }
        if repeatMode == .one {
            seek(to: 0)
            av.play()
        } else if preloaded == nil {
            // The next one wasn't ready (or the queue ended).
            if let n = nextIndex() {
                index = n
                load()
            } else {
                self.loaded = nil
                forgetItem(item)
                position = 0
                isPlaying = false
                updateNowPlaying()
            }
        }
    }

    /// A URL that stopped working (expired, 403, network switched to
    /// another IP): resolve once more and continue where it stopped.
    private func itemFailed(_ item: AVPlayerItem?, _ error: Error?) {
        guard let item else { return }
        if let preloaded, preloaded.item === item {
            dropPreload()
            Task { await Services.resolver.forget(preloaded.videoId) }
            return
        }
        guard let loaded, loaded.item === item, let track = current else { return }
        if retried.insert(loaded.videoId).inserted {
            let at = position
            Task {
                await Services.resolver.forget(loaded.videoId)
                load(resumeAt: at)
            }
        } else {
            self.loaded = nil
            failed(track, error ?? YTMError.noStream)
        }
    }

    private func tick(_ time: CMTime) {
        guard loaded != nil else { return }
        position = time.seconds.isFinite ? time.seconds : 0
    }

    // MARK: System integration

    private func setUpAudioSession() {
        #if os(iOS)
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .default, policy: .longFormAudio)
        NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil,
                                               queue: .main) { [weak self] note in
            let info = note.userInfo ?? [:]
            let type = (info[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init)
            let options = AVAudioSession.InterruptionOptions(rawValue: info[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0)
            guard type == .ended, options.contains(.shouldResume) else { return }
            MainActor.assumeIsolated { self?.av.play() }
        }
        #endif
    }

    private func activateAudioSession() {
        #if os(iOS)
        try? AVAudioSession.sharedInstance().setActive(true)
        #endif
    }

    private func setUpRemoteCommands() {
        let commands = MPRemoteCommandCenter.shared()
        commands.playCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.isPlaying == false { self?.togglePlay() } }
            return .success
        }
        commands.pauseCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { self?.av.pause() }
            return .success
        }
        commands.togglePlayPauseCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { self?.togglePlay() }
            return .success
        }
        commands.nextTrackCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { self?.next() }
            return .success
        }
        commands.previousTrackCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { self?.previous() }
            return .success
        }
        commands.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            let at = event.positionTime
            MainActor.assumeIsolated { self?.seek(to: at) }
            return .success
        }
    }

    private func updateNowPlaying() {
        let center = MPNowPlayingInfoCenter.default()
        guard let track = current else {
            center.nowPlayingInfo = nil
            return
        }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: track.title,
            MPMediaItemPropertyArtist: track.artist,
            MPMediaItemPropertyPlaybackDuration: duration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: position,
            MPNowPlayingInfoPropertyPlaybackRate: isPlaying ? 1.0 : 0.0,
        ]
        if let artwork, artwork.videoId == track.videoId {
            info[MPMediaItemPropertyArtwork] = artwork.value
        } else {
            loadArtwork(track)
        }
        center.nowPlayingInfo = info
    }

    private func loadArtwork(_ track: Track) {
        Task {
            guard let url = track.largeThumbnailURL,
                  let (data, _) = try? await URLSession.shared.data(from: url),
                  let image = squareCover(data), current?.videoId == track.videoId else { return }
            artwork = (track.videoId, image)
            updateNowPlaying()
        }
    }

    // MARK: Persistence

    private struct SavedQueue: Codable {
        var tracks: [Track]
        var index: Int
    }

    private static var queueFile: URL { Library.directory.appendingPathComponent("queue.json") }

    private func saveQueue() {
        let saved = SavedQueue(tracks: Array(queue.prefix(500)), index: index)
        if let data = try? JSONEncoder().encode(saved) {
            try? data.write(to: Self.queueFile, options: .atomic)
        }
    }

    /// Last session's queue, paused at its song.
    private func restoreQueue() {
        guard let data = try? Data(contentsOf: Self.queueFile),
              let saved = try? JSONDecoder().decode(SavedQueue.self, from: data),
              saved.tracks.indices.contains(saved.index) else { return }
        queue = saved.tracks
        index = saved.index
        duration = Double(current?.durationSecs ?? 0)
    }
}
