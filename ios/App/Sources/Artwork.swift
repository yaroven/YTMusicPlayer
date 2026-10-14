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
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
    let w = image.width, h = image.height
    // 4:3 with bars: the picture is the middle 16:9 band.
    let band = abs(Double(w) / Double(h) - 4.0 / 3.0) < 0.01 ? w * 9 / 16 : h
    let side = min(w, band)
    guard let square = image.cropping(to: CGRect(x: (w - side) / 2, y: (h - side) / 2, width: side, height: side))
    else { return nil }
    #if canImport(UIKit)
    let picture = UIImage(cgImage: square)
    #else
    let picture = NSImage(cgImage: square, size: NSSize(width: side, height: side))
    #endif
    return MPMediaItemArtwork(boundsSize: picture.size) { _ in picture }
}
