import Foundation
import YTMKit

/// Songs and their clips found so far (YouTube Music's "Song" / "Video",
/// `Catalog.mediaPair`): what plays as a track's sound (the studio song,
/// also for a clip) and which clip "Video" shows. In memory, a few hundred.
actor Pairs {
    static let shared = Pairs()
    private var known: [String: (pair: MediaPair, complete: Bool)] = [:]

    /// The id whose sound plays for `track`: its studio song (one request
    /// the first time); the track's own id when the catalog can't tell.
    func audioId(_ track: Track) async -> String {
        await pair(track, findClip: false)?.song ?? track.videoId
    }

    /// The song and its clip; `findClip` also looks for the clip of a song.
    func pair(_ track: Track, findClip: Bool) async -> MediaPair? {
        if let (pair, complete) = known[track.videoId], complete || !findClip || pair.video != nil {
            return pair
        }
        guard let pair = try? await Services.catalog.mediaPair(videoId: track.videoId, title: track.title,
                                                              artist: track.artist, findClip: findClip)
        else { return nil }
        if known.count >= 500 { known.removeAll() }
        known[track.videoId] = (pair, findClip)
        return pair
    }
}
