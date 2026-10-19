import SwiftUI
import YTMKit

/// Album, artist and playlist pages: the last ones opened (to open again at
/// once), the ones loading since a finger touched their card (prefetch),
/// and the cards themselves (so a page shows its title and art before it
/// has loaded).
@MainActor
final class PageCache {
    static let shared = PageCache()
    static let size = 12

    /// Newest last.
    private var pages: [(id: String, page: Page)] = []
    private var loading: [String: Task<Page, Error>] = [:]
    private var previews: [String: Item] = [:]

    func cached(_ id: String) -> Page? {
        pages.last { $0.id == id }?.page
    }

    func preview(_ id: String) -> Item? {
        previews[id]
    }

    /// A card or row is on screen: its title and art are what its page
    /// shows first.
    func note(_ item: Item) {
        guard item.track == nil, previews[item.browseId] == nil else { return }
        if previews.count >= 300 { previews = [:] }
        previews[item.browseId] = item
    }

    /// A card was touched: start loading its page.
    func prefetch(_ item: Item) {
        guard item.track == nil else { return }
        note(item)
        guard cached(item.browseId) == nil, loading[item.browseId] == nil else { return }
        _ = start(item.browseId)
    }

    /// The page's first answer: from here, from a prefetch under way, or
    /// from YouTube Music.
    func load(_ id: String) async throws -> Page {
        if let page = cached(id) { return page }
        return try await (loading[id] ?? start(id)).value
    }

    /// Keeps `page` (also when more of its songs arrived).
    func remember(_ id: String, _ page: Page) {
        pages.removeAll { $0.id == id }
        pages.append((id, page))
        if pages.count > Self.size { pages.removeFirst(pages.count - Self.size) }
    }

    private func start(_ id: String) -> Task<Page, Error> {
        let task = Task { try await Services.catalog.page(id, full: false) }
        loading[id] = task
        Task {
            if let page = try? await task.value { remember(id, page) }
            loading[id] = nil
        }
        return task
    }
}

/// A plain button look that starts loading the card's page as soon as a
/// finger is down on it (before the tap ends).
struct PrefetchOnPress: ButtonStyle {
    let item: Item

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1)
            .onChange(of: configuration.isPressed) { _, pressed in
                if pressed { PageCache.shared.prefetch(item) }
            }
    }
}

/// A grey placeholder row while songs load.
struct SkeletonRow: View {
    @State private var dim = false

    var body: some View {
        HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 5).frame(width: 48, height: 48)
            VStack(alignment: .leading, spacing: 6) {
                Capsule().frame(width: 180, height: 12)
                Capsule().frame(width: 110, height: 10)
            }
            Spacer()
        }
        .foregroundStyle(.white.opacity(dim ? 0.06 : 0.12))
        .onAppear {
            withAnimation(.easeInOut(duration: 0.8).repeatForever()) { dim = true }
        }
        .accessibilityHidden(true)
    }
}
