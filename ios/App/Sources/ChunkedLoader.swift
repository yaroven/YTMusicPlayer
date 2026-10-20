import AVFoundation
import Foundation

/// googlevideo throttles big ranges to the song's own bitrate: measured
/// 2026-10-20, an open-ended request for a 23 MB song came at ~32 KB/s (a
/// 39 MB one took 20 minutes), while 1 MB ranges of the same URL came at
/// ~10 MB/s. Songs under ~10 MB were never throttled, so only long ones
/// wouldn't start. AVPlayer asks for big ranges itself, so the asset gets a
/// URL with our own scheme and this delegate serves each of its requests as
/// a run of `chunk`-sized range requests (the desktop player reads 256 KB
/// ones, yt-dlp 10 MB).
final class ChunkedLoader: NSObject, AVAssetResourceLoaderDelegate, @unchecked Sendable {
    static let scheme = "ytmchunk"
    static let queue = DispatchQueue(label: "chunked-loader")
    static let chunk = 1 << 20

    let url: URL
    private let knownLength: Int?
    private let contentType: String
    private let session: URLSession = {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 20
        return URLSession(configuration: config)
    }()
    private let lock = NSLock()
    private var tasks: [ObjectIdentifier: Task<Void, Never>] = [:]
    private var length: Int?

    /// `video`: a muxed MP4 (itag 18), not an audio-only one.
    init(url: URL, length: Int?, video: Bool) {
        self.url = url
        knownLength = length
        contentType = video ? AVFileType.mp4.rawValue : AVFileType.m4a.rawValue
    }

    /// What the asset is opened with: the same URL under our scheme.
    var assetURL: URL {
        var parts = URLComponents(url: url, resolvingAgainstBaseURL: false)!
        parts.scheme = Self.scheme
        return parts.url!
    }

    func resourceLoader(_ resourceLoader: AVAssetResourceLoader,
                        shouldWaitForLoadingOfRequestedResource request: AVAssetResourceLoadingRequest) -> Bool {
        let key = ObjectIdentifier(request)
        let task = Task { [weak self] in
            await self?.serve(request)
            self?.lock.withLock { _ = self?.tasks.removeValue(forKey: key) }
        }
        lock.withLock { tasks[key] = task }
        return true
    }

    func resourceLoader(_ resourceLoader: AVAssetResourceLoader, didCancel request: AVAssetResourceLoadingRequest) {
        lock.withLock { tasks.removeValue(forKey: ObjectIdentifier(request)) }?.cancel()
    }

    /// Stops everything (the item is gone).
    func cancelAll() {
        let running = lock.withLock { () -> [Task<Void, Never>] in
            defer { tasks = [:] }
            return Array(tasks.values)
        }
        running.forEach { $0.cancel() }
    }

    private func serve(_ request: AVAssetResourceLoadingRequest) async {
        do {
            let total = try await totalLength()
            if let info = request.contentInformationRequest {
                info.contentType = contentType
                info.contentLength = Int64(total)
                info.isByteRangeAccessSupported = true
            }
            guard let data = request.dataRequest else {
                request.finishLoading()
                return
            }
            var offset = Int(data.currentOffset)
            let end = data.requestsAllDataToEndOfResource
                ? total : min(total, Int(data.requestedOffset) + data.requestedLength)
            while offset < end {
                try Task.checkCancellation()
                let upTo = min(offset + Self.chunk, end)
                let bytes = try await fetch(offset, upTo - 1)
                guard !bytes.isEmpty else { break }
                data.respond(with: bytes)
                offset += bytes.count
            }
            request.finishLoading()
        } catch is CancellationError {
        } catch {
            request.finishLoading(with: error)
        }
    }

    private func fetch(_ from: Int, _ to: Int) async throws -> Data {
        var request = URLRequest(url: url)
        request.setValue("bytes=\(from)-\(to)", forHTTPHeaderField: "Range")
        let (bytes, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 206 || status == 200 else {
            // 403 / 410: the player's item fails and gets a fresh URL.
            throw URLError(.badServerResponse, userInfo: [NSLocalizedDescriptionKey: "googlevideo: HTTP \(status)"])
        }
        return bytes
    }

    /// The file's size: from the player response, else from a 1-byte range.
    private func totalLength() async throws -> Int {
        if let known = knownLength ?? lock.withLock({ length }) { return known }
        var request = URLRequest(url: url)
        request.setValue("bytes=0-0", forHTTPHeaderField: "Range")
        let (_, response) = try await session.data(for: request)
        guard let range = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Range"),
              let total = range.split(separator: "/").last.flatMap({ Int($0) }) else {
            throw URLError(.badServerResponse)
        }
        lock.withLock { length = total }
        return total
    }
}
