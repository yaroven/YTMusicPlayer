import Foundation
import SwiftUI
import YTMKit
#if canImport(WebKit)
import WebKit
#endif

/// The signed-in Google account: who it is, sign in / out.
@MainActor @Observable
final class Account {
    private(set) var signedIn = Services.auth.signedIn
    private(set) var name: String?

    init() {
        if signedIn { loadName() }
    }

    func signIn(with cookies: [String: String]) {
        Keychain.save(cookies)
        Services.auth.set(cookies)
        signedIn = true
        Task { await Services.resolver.reset() }
        loadName()
    }

    func signOut() {
        Keychain.delete()
        Services.auth.set([:])
        signedIn = false
        name = nil
        Task { await Services.resolver.reset() }
    }

    private func loadName() {
        Task { name = try? await Services.catalog.accountName() }
    }
}

/// Google's own sign-in page in a web view (the password goes only to
/// Google). Once youtube.com has a session, its cookies are taken and the
/// web view's storage is thrown away.
struct SignInView: View {
    @Environment(Account.self) private var account
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                #if os(iOS)
                GoogleWebView { cookies in
                    account.signIn(with: cookies)
                    dismiss()
                }
                .ignoresSafeArea(edges: .bottom)
                #else
                Text("Sign-in runs on iPhone")
                #endif
            }
            .navigationTitle("Google account")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }
}

#if os(iOS)
private struct GoogleWebView: UIViewRepresentable {
    /// After sign-in Google sends the browser on to YouTube Music.
    static let start = URL(string: "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Dm%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F")!
    /// Google refuses sign-in in web views that don't look like Safari.
    static let safari = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1"

    let done: ([String: String]) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: config)
        view.customUserAgent = Self.safari
        view.navigationDelegate = context.coordinator
        view.load(URLRequest(url: Self.start))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    final class Coordinator: NSObject, WKNavigationDelegate {
        let done: ([String: String]) -> Void
        private var finished = false

        init(done: @escaping ([String: String]) -> Void) {
            self.done = done
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            guard let host = webView.url?.host, host.hasSuffix("youtube.com") else { return }
            webView.configuration.websiteDataStore.httpCookieStore.getAllCookies { [weak self] all in
                var cookies: [String: String] = [:]
                for cookie in all where cookie.domain.hasSuffix("youtube.com") {
                    cookies[cookie.name] = cookie.value
                }
                guard let self, !self.finished, Auth.isSignedIn(cookies) else { return }
                self.finished = true
                DispatchQueue.main.async { self.done(cookies) }
            }
        }
    }
}
#endif

/// Which client plays an age-restricted video with this account, and
/// whether the audio downloads. Its output tells how to edit ios/clients.json.
struct PlaybackTestView: View {
    @State private var videoId = "Tq92D6wQ1mg"
    @State private var lines: [String] = []
    @State private var running = false

    var body: some View {
        List {
            Section {
                TextField("Video id or link", text: $videoId)
                    .autocorrectionDisabled()
                Button(running ? "Testing…" : "Run test") { run() }
                    .disabled(running)
            } footer: {
                Text("The default video is age-restricted. Send a screenshot of the result to the developer.")
            }
            if !lines.isEmpty {
                Section("Result") {
                    ForEach(lines, id: \.self) { Text($0).font(.footnote.monospaced()) }
                }
            }
        }
        .navigationTitle("Age-restricted test")
    }

    private func run() {
        let id = videoIdIn(videoId)
        running = true
        lines = []
        Task {
            lines = await Services.resolver.diagnose(id)
            running = false
        }
    }
}

/// "https://music.youtube.com/watch?v=ID&list=…" or "youtu.be/ID" → "ID".
func videoIdIn(_ text: String) -> String {
    let text = text.trimmingCharacters(in: .whitespaces)
    if let v = URLComponents(string: text)?.queryItems?.first(where: { $0.name == "v" })?.value { return v }
    if let url = URL(string: text), url.host?.hasSuffix("youtu.be") == true { return String(url.path.dropFirst()) }
    return text
}

/// The signed-in account's YouTube Music library.
struct AccountLibraryView: View {
    @State private var playlists: [Item] = []
    @State private var error: String?

    var body: some View {
        List {
            Section {
                NavigationLink {
                    RemoteTracksView(title: "Liked music") { try await Services.catalog.likedSongs() }
                } label: {
                    Label("Liked music", systemImage: "hand.thumbsup.fill")
                }
            }
            Section("Playlists") {
                ForEach(playlists) { ItemRow(item: $0) }
            }
        }
        .overlay {
            if let error, playlists.isEmpty {
                ContentUnavailableView("Can't load", systemImage: "exclamationmark.triangle", description: Text(error))
            }
        }
        .navigationTitle("YouTube Music")
        .task {
            do {
                playlists = try await Services.catalog.libraryPlaylists()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// Songs fetched once from YouTube Music.
struct RemoteTracksView: View {
    let title: String
    let load: () async throws -> [Track]
    @State private var tracks: [Track]?
    @State private var error: String?
    @Environment(Player.self) private var player

    var body: some View {
        List {
            if let tracks, !tracks.isEmpty {
                HStack {
                    Button { player.play(tracks) } label: {
                        Label("Play", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button { player.play(tracks.shuffled()) } label: {
                        Label("Shuffle", systemImage: "shuffle").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                .listRowSeparator(.hidden)
                ForEach(Array(tracks.enumerated()), id: \.offset) { i, track in
                    TrackRow(track: track, list: tracks, index: i)
                }
            }
        }
        .listStyle(.plain)
        .overlay {
            if let error {
                ContentUnavailableView("Can't load", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if tracks == nil {
                ProgressView()
            } else if tracks?.isEmpty == true {
                ContentUnavailableView("No songs", systemImage: "music.note")
            }
        }
        .navigationTitle(title)
        .task {
            guard tracks == nil else { return }
            do {
                tracks = try await load()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
