import SwiftUI
import YTMKit

@main
struct YTMApp: App {
    @State private var library: Library
    @State private var player: Player
    @State private var router = Router()
    @State private var account = Account()

    init() {
        // Thumbnails: small memory cache, the rest on disk.
        URLCache.shared = URLCache(memoryCapacity: 8 << 20, diskCapacity: 100 << 20)
        let library = Library()
        _library = State(initialValue: library)
        _player = State(initialValue: Player(library: library))
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(library)
                .environment(player)
                .environment(router)
                .environment(account)
                .tint(.accentColor)
                .preferredColorScheme(.dark)
        }
    }
}
