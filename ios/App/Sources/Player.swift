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
    private var resolving = false { didSet { watchSlow() } }
    private var waiting = false { didSet { watchSlow() } }
    /// Buffering for a while: the connection is slow.
    private(set) var slow = false
    /// Playback stopped because the connection went; it carries on (where
    /// it stopped) when the connection is back, instead of skipping songs.
    private(set) var waitingForNetwork = false
    @ObservationIgnored private var slowTask: Task<Void, Never>?
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
    /// Seconds the end of a song and the start of the next overlap
    /// (0 = gapless, back to back).
    var crossfade = UserDefaults.standard.object(forKey: "crossfade") as? Double ?? 0 {
        didSet {
            UserDefaults.standard.set(crossfade, forKey: "crossfade")
            // The preloaded song sits in the queue (gapless) or waits (fade).
            queueChanged()
        }
    }
    /// Turn loud masters down to YouTube's loudness target.
    var normalize = UserDefaults.standard.object(forKey: "normalize") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(normalize, forKey: "normalize")
            av.volume = loaded.map(gain) ?? 1
        }
    }

    /// Sleep timer: pause at this moment, or at the end of the song.
    private(set) var sleepUntil: Date?
    private(set) var sleepAtEndOfSong = false

    var current: Track? { queue.indices.contains(index) ? queue[index] : nil }
    /// The player's own clock, between the half-second `position` updates
    /// (karaoke lyrics read it ten times a second).
    var exactPosition: TimeInterval {
        let t = av.currentTime().seconds
        return t.isFinite && loaded != nil ? t : position
    }
    var upNext: ArraySlice<Track> { queue.dropFirst(index + 1) }

    private struct Loaded {
        let index: Int
        let videoId: String
        let item: AVPlayerItem
        let loudnessDb: Double?
        let duration: TimeInterval?
    }

    @ObservationIgnored private let library: Library
    /// The player of the song being heard. During a crossfade the next song
    /// plays in `incoming`, which then takes this place.
    @ObservationIgnored private var av = AVQueuePlayer()
    @ObservationIgnored private var incoming: AVQueuePlayer?
    @ObservationIgnored private var fadeTask: Task<Void, Never>?
    @ObservationIgnored private let muted: Bool
    @ObservationIgnored private var loaded: Loaded?
    @ObservationIgnored private var preloaded: Loaded?
    @ObservationIgnored private var loadTask: Task<Void, Never>?
    @ObservationIgnored private var preloadTask: Task<Void, Never>?
    @ObservationIgnored private var observations: [NSKeyValueObservation] = []
    @ObservationIgnored private var itemObservations: [ObjectIdentifier: NSKeyValueObservation] = [:]
    @ObservationIgnored private var videoObservations: [ObjectIdentifier: NSKeyValueObservation] = [:]
    @ObservationIgnored private var timeObserver: Any?
    /// Songs whose URL was already refreshed once after a failure.
    @ObservationIgnored private var retried: Set<String> = []
    @ObservationIgnored private var failuresInARow = 0
    @ObservationIgnored private var radioSeed: String?
    @ObservationIgnored private var artwork: (videoId: String, value: MPMediaItemArtwork)?

    /// `muted`: for the macOS player check (`ios/player-check.sh`).
    init(library: Library, muted: Bool = false) {
        self.library = library
        self.muted = muted
        av.isMuted = muted
        restoreQueue()
        Connectivity.shared.onReconnect { [weak self] in self?.networkBack() }
        setUpPlayer()
        setUpAudioSession()
        setUpRemoteCommands()
    }

    /// What the player is waiting for, or what went wrong (one line).
    var statusText: String? {
        if let error { return error }
        if waitingForNetwork { return String(localized: "No internet connection. Playback continues when it's back.") }
        if slow, isPlaying {
            return Connectivity.shared.online ? String(localized: "Slow connection, buffering…")
                : String(localized: "No internet connection. Playback continues when it's back.")
        }
        return nil
    }

    // MARK: Commands

    /// Plays `tracks` from `start`. Offline only downloaded songs can play:
    /// the downloaded part of the list, from this song.
    func playFrom(_ tracks: [Track], at start: Int) {
        guard tracks.indices.contains(start) else { return }
        guard !Connectivity.shared.online else { return play(tracks, at: start) }
        let track = tracks[start]
        guard library.isDownloaded(track.videoId) else {
            Notice.shared.show(String(localized: "No internet: only downloaded songs play offline"))
            return
        }
        let offline = tracks.filter { library.isDownloaded($0.videoId) }
        play(offline, at: offline.firstIndex { $0.videoId == track.videoId } ?? 0)
    }

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
        settleFade()
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
        settleFade()
        guard let n = nextIndex() else { return }
        if let preloaded, preloaded.index == n, av.items().count > 1 {
            av.advanceToNextItem()
        } else {
            index = n
            load()
        }
    }

    func previous() {
        settleFade()
        if position > 3 || index == 0 {
            seek(to: 0)
        } else {
            index -= 1
            load()
        }
    }

    func seek(to seconds: TimeInterval) {
        settleFade()
        position = seconds
        av.seek(to: CMTime(seconds: seconds, preferredTimescale: 600)) { [weak self] _ in
            Task { @MainActor in self?.updateNowPlaying() }
        }
        updateNowPlaying()
    }

    /// Pauses in `minutes`, or at the end of this song when `nil`.
    func setSleep(minutes: Int?) {
        if let minutes {
            sleepUntil = Date().addingTimeInterval(TimeInterval(minutes * 60))
            sleepAtEndOfSong = false
        } else {
            sleepUntil = nil
            sleepAtEndOfSong = true
        }
    }

    func cancelSleep() {
        sleepUntil = nil
        sleepAtEndOfSong = false
    }

    /// The clip is on screen: the audio waits.
    func pauseForVideo() -> TimeInterval {
        let at = exactPosition
        if resolving { playWhenReady = false }
        av.pause()
        return at
    }

    /// Back from the clip: the song carries on where the video stopped.
    func resumeAfterVideo(at seconds: TimeInterval?) {
        guard loaded != nil else { return load(resumeAt: seconds ?? position) }
        if let seconds { seek(to: seconds) }
        activateAudioSession()
        av.play()
    }

    /// Dislikes the song (YouTube learns from it) and moves on if it plays.
    func dislike(_ track: Track) {
        library.dislike(track)
        if current?.videoId == track.videoId { next() }
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
        waitingForNetwork = false
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
        if let (url, saved) = library.download(of: track.videoId) {
            // On the phone: no network at all.
            let item = AVPlayerItem(url: url)
            let duration = saved.duration ?? track.durationSecs.map(Double.init)
            if let duration {
                item.forwardPlaybackEndTime = CMTime(seconds: duration, preferredTimescale: 1000)
            }
            watch(item)
            return Loaded(index: index, videoId: track.videoId, item: item, loudnessDb: saved.loudnessDb,
                          duration: duration)
        }
        let stream = try await Services.resolver.stream(track.videoId)
        try Task.checkCancellation()
        let item = AVPlayerItem(url: stream.url)
        if stream.hasVideo {
            // A muxed MP4 (age-restricted songs): decode only its audio.
            videoObservations[ObjectIdentifier(item)] = item.observe(\.tracks, options: [.new]) { item, _ in
                for track in item.tracks where track.assetTrack?.mediaType == .video { track.isEnabled = false }
            }
        }
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
        if error.isNetwork {
            // Don't run through the queue while the connection is down:
            // wait here and carry on when it comes back.
            waitingForNetwork = true
            self.error = nil
            updateNowPlaying()
            if Connectivity.shared.online {
                // Online but too slow: try again in a moment.
                let target = index
                Task {
                    try? await Task.sleep(for: .seconds(5))
                    guard index == target, waitingForNetwork else { return }
                    networkBack()
                }
            }
            return
        }
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
            // Crossfading, the next song starts in its own player instead.
            if crossfade == 0 { av.insert(next.item, after: loaded.item) }
        }
    }

    private func dropPreload() {
        preloadTask?.cancel()
        cancelFade()
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

    /// The connection is back: the song that was waiting for it goes on.
    private func networkBack() {
        guard waitingForNetwork else { return }
        waitingForNetwork = false
        load(resumeAt: position)
    }

    private func watchSlow() {
        if isBuffering {
            guard slowTask == nil else { return }
            slowTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(4))
                guard let self, !Task.isCancelled, isBuffering else { return }
                slow = true
            }
        } else {
            slowTask?.cancel()
            slowTask = nil
            if slow { slow = false }
        }
    }

    // MARK: AVPlayer events

    private func setUpPlayer() {
        attach(av)
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

    /// Listens to the player whose song is being heard.
    private func attach(_ av: AVQueuePlayer) {
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
    }

    private func detach(_ av: AVQueuePlayer) {
        observations.forEach { $0.invalidate() }
        observations = []
        if let timeObserver { av.removeTimeObserver(timeObserver) }
        timeObserver = nil
    }

    // MARK: Crossfade

    /// The song is `crossfade` seconds from its end and the next one is
    /// ready: start the next in its own player and swap the volumes over.
    private func startFadeIfDue() {
        guard crossfade > 0, incoming == nil, isPlaying, let loaded, let next = preloaded,
              let length = loaded.duration ?? (duration > 0 ? duration : nil),
              length - position <= crossfade else { return }
        let player = AVQueuePlayer(items: [next.item])
        player.isMuted = muted
        player.volume = 0
        player.play()
        incoming = player
        let (from, to) = (gain(loaded), gain(next))
        let span = max(0.5, min(crossfade, length - position))
        let start = Date()
        fadeTask = Task { [weak self] in
            while !Task.isCancelled {
                let t = min(1, Date().timeIntervalSince(start) / span)
                guard let self else { return }
                self.av.volume = from * Float(1 - t)
                player.volume = to * Float(t)
                if t >= 1 { break }
                try? await Task.sleep(for: .milliseconds(50))
            }
            guard !Task.isCancelled else { return }
            self?.finishFade()
        }
    }

    /// The incoming song becomes the one being heard.
    private func finishFade() {
        guard let player = incoming, let next = preloaded else { return }
        fadeTask?.cancel()
        fadeTask = nil
        incoming = nil
        let old = av
        detach(old)
        old.pause()
        old.removeAllItems()
        forgetItem(loaded?.item)
        av = player
        attach(av)
        loaded = next
        preloaded = nil
        index = next.index
        av.volume = gain(next)
        let t = av.currentTime().seconds
        position = t.isFinite ? t : 0
        duration = next.duration ?? Double(current?.durationSecs ?? 0)
        retried.remove(next.videoId)
        saveQueue()
        if let track = current { started(track) }
    }

    /// Something else happened mid-fade (pause, seek, another song): finish
    /// the swap at once so there is one player again.
    private func settleFade() {
        if incoming != nil { finishFade() }
    }

    /// The fade is abandoned (a new song loads): the incoming player stops.
    private func cancelFade() {
        fadeTask?.cancel()
        fadeTask = nil
        incoming?.pause()
        incoming?.removeAllItems()
        incoming = nil
    }

    private func watch(_ item: AVPlayerItem) {
        itemObservations[ObjectIdentifier(item)] = item.observe(\.status, options: [.new]) { [weak self] item, _ in
            guard item.status == .failed else { return }
            let error = item.error
            Task { @MainActor in self?.itemFailed(item, error) }
        }
    }

    private func forgetItem(_ item: AVPlayerItem?) {
        if let item {
            itemObservations[ObjectIdentifier(item)] = nil
            videoObservations[ObjectIdentifier(item)] = nil
        }
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
        if sleepAtEndOfSong {
            // The next song already started (gapless): stop it at its start.
            sleepAtEndOfSong = false
            av.pause()
            av.seek(to: .zero)
        }
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
        if incoming != nil { return finishFade() }
        if sleepAtEndOfSong, preloaded == nil {
            sleepAtEndOfSong = false
            av.pause()
            isPlaying = false
            updateNowPlaying()
            return
        }
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
        if let until = sleepUntil, Date() >= until {
            sleepUntil = nil
            av.pause()
        }
        startFadeIfDue()
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
            // maxresdefault is the sharpest, but not every video has one.
            let urls = [track.largeThumbnailURL, track.fallbackThumbnailURL].compactMap { $0 }
            for url in urls {
                guard let (data, response) = try? await URLSession.shared.data(from: url),
                      (response as? HTTPURLResponse)?.statusCode == 200,
                      let image = squareCover(data) else { continue }
                guard current?.videoId == track.videoId else { return }
                artwork = (track.videoId, image)
                updateNowPlaying()
                return
            }
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
