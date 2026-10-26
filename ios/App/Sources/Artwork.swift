import ImageIO
import MediaPlayer
import SwiftUI
import YTMKit

enum ArtworkSource: Hashable {
    case video(String)
    case url(URL)
    case none
}

extension Item {
    var artwork: ArtworkSource {
        if let track { return .video(track.videoId) }
        return thumbnail.map(ArtworkSource.url) ?? .none
    }
}

/// googleusercontent thumbnails take their size in the URL (`=w226-h226…`).
func resized(_ url: URL, _ pixels: Int) -> URL {
    let s = url.absoluteString
    guard s.contains("googleusercontent.com"), let cut = s.range(of: "=w", options: .backwards) else { return url }
    return URL(string: s[..<cut.lowerBound] + "=w\(pixels)-h\(pixels)-l90-rj") ?? url
}

/// Square cover art. Video thumbnails are 16:9 (`mqdefault`,
/// `maxresdefault`) or 16:9 inside a letterboxed 4:3 picture (`hqdefault`);
/// both show the centre square. Big covers ask for `maxresdefault` (720 px
/// of square, nothing upscaled) and drop to `hqdefault` where YouTube has
/// no such picture.
struct Artwork: View {
    let source: ArtworkSource
    var size: CGFloat
    var round = false

    var body: some View {
        ZStack {
            Rectangle().fill(Color.white.opacity(0.08))
            if let url {
                AsyncImage(url: url, transaction: Transaction(animation: .easeOut(duration: 0.25))) { phase in
                    if let image = phase.image {
                        picture(image, letterboxed: letterboxed)
                    } else if phase.error != nil, let fallback {
                        AsyncImage(url: fallback) { image in
                            picture(image, letterboxed: true)
                        } placeholder: {
                            EmptyView()
                        }
                    }
                }
            } else {
                Image(systemName: "music.note").foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .clipShape(round ? AnyShape(Circle()) : AnyShape(RoundedRectangle(cornerRadius: size > 100 ? 10 : 5)))
    }

    private func picture(_ image: Image, letterboxed: Bool) -> some View {
        letterboxed
            ? AnyView(image.resizable().frame(width: size * 16 / 9, height: size * 4 / 3))
            : AnyView(image.resizable().scaledToFill())
    }

    private var url: URL? {
        switch source {
        case let .video(id):
            // A downloaded song's own cover, also offline.
            if let local = Covers.local(id) { return local }
            // The album's square art, when a lookup found it (any size).
            if let cover = Pairs.cover(id).flatMap(URL.init(string:)) { return resized(cover, Int(size * 3)) }
            let name = size > 240 ? "maxresdefault" : size > 120 ? "hqdefault" : "mqdefault"
            return URL(string: "https://i.ytimg.com/vi/\(id)/\(name).jpg")
        case let .url(url): return resized(url, Int(size * 3))
        case .none: return nil
        }
    }

    /// Where a missing `maxresdefault` falls back to.
    private var fallback: URL? {
        guard case let .video(id) = source else { return nil }
        return URL(string: "https://i.ytimg.com/vi/\(id)/hqdefault.jpg")
    }

    private var letterboxed: Bool {
        guard let url, url.host == "i.ytimg.com" else { return false }
        return url.path.contains("hqdefault") || url.path.contains("sddefault")
    }
}

/// Lock screen art: the centre square of a (letterboxed) video thumbnail.
func squareCover(_ data: Data) -> MPMediaItemArtwork? {
    guard let square = squareImage(data) else { return nil }
    let side = square.width
    #if canImport(UIKit)
    let picture = UIImage(cgImage: square)
    #else
    let picture = NSImage(cgImage: square, size: NSSize(width: side, height: side))
    #endif
    return MPMediaItemArtwork(boundsSize: picture.size) { _ in picture }
}

/// The centre square of a (letterboxed) video thumbnail.
func squareImage(_ data: Data) -> CGImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
    let w = image.width, h = image.height
    // 4:3 with bars: the picture is the middle 16:9 band.
    let band = abs(Double(w) / Double(h) - 4.0 / 3.0) < 0.01 ? w * 9 / 16 : h
    let side = min(w, band)
    return image.cropping(to: CGRect(x: (w - side) / 2, y: (h - side) / 2, width: side, height: side))
}

/// Covers of downloaded songs, kept next to them so the lock screen and the
/// lists show them offline: Application Support/covers/<video id>.jpg,
/// the centre square at up to 544 px (~90 KB each).
enum Covers {
    static let directory: URL = {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let dir = support.appendingPathComponent("covers", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }()

    /// Which covers are on disk (read once, then kept up to date).
    nonisolated(unsafe) private static var saved: Set<String> = {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
        return Set(names.filter { $0.hasSuffix(".jpg") }.map { String($0.dropLast(4)) })
    }()
    private static let lock = NSLock()

    static func file(_ videoId: String) -> URL { directory.appendingPathComponent("\(videoId).jpg") }

    /// The cover's file, when this song's cover is on the phone.
    static func local(_ videoId: String) -> URL? {
        lock.lock()
        defer { lock.unlock() }
        return saved.contains(videoId) ? file(videoId) : nil
    }

    /// Fetches and keeps the song's cover: the album's own art first, else
    /// the clip's frame (maxresdefault, else hqdefault).
    static func fetch(_ track: Track) async {
        guard local(track.videoId) == nil else { return }
        let album = await Pairs.shared.pair(track, findClip: false)?.cover
            .flatMap(URL.init(string:)).map { resized($0, 1200) }
        for url in [album, track.largeThumbnailURL, track.fallbackThumbnailURL].compactMap({ $0 }) {
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode == 200,
                  let jpeg = squareJPEG(data, side: 544) else { continue }
            guard (try? jpeg.write(to: file(track.videoId), options: .atomic)) != nil else { return }
            lock.lock()
            saved.insert(track.videoId)
            lock.unlock()
            return
        }
    }

    static func remove(_ videoId: String) {
        try? FileManager.default.removeItem(at: file(videoId))
        lock.lock()
        saved.remove(videoId)
        lock.unlock()
    }

    /// The centre square, scaled down to `side` px, as JPEG.
    private static func squareJPEG(_ data: Data, side: Int) -> Data? {
        guard let square = squareImage(data) else { return nil }
        let out = min(side, square.width)
        guard let context = CGContext(data: nil, width: out, height: out, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        context.interpolationQuality = .high
        context.draw(square, in: CGRect(x: 0, y: 0, width: out, height: out))
        guard let scaled = context.makeImage() else { return nil }
        let result = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(result, "public.jpeg" as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, scaled, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return result as Data
    }
}
