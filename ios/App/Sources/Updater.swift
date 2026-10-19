import SwiftUI

/// A newer release: its version, its .ipa and its page.
struct AppUpdate: Equatable {
    var version: String
    var ipa: URL
    var page: URL
}

/// "There's a newer version": GitHub's latest release, at most once a day.
/// An iPhone app can't install itself; SideStore does, from the .ipa this
/// hands it (`sidestore://install?url=…`, as AltStore's links), else the
/// release page opens.
@MainActor @Observable
final class Updater {
    static let shared = Updater()
    static let latest = URL(string: "https://api.github.com/repos/yaroven/YTMusicPlayer/releases/latest")!

    private(set) var available: AppUpdate?
    private(set) var checking = false
    /// After a check the user asked for.
    private(set) var upToDate = false
    var dismissed = false

    var current: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }

    func checkDaily() {
        let last = UserDefaults.standard.double(forKey: "updateChecked")
        guard Date().timeIntervalSince1970 - last > 24 * 3600 else { return }
        Task { await check(quiet: true) }
    }

    func check(quiet: Bool = false) async {
        checking = true
        upToDate = false
        defer { checking = false }
        var request = URLRequest(url: Self.latest, timeoutInterval: 15)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let release = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let tag = release["tag_name"] as? String,
              let page = (release["html_url"] as? String).flatMap(URL.init(string:)) else {
            if !quiet { Notice.shared.show(String(localized: "Couldn't check for updates")) }
            return
        }
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: "updateChecked")
        let version = tag.hasPrefix("v") ? String(tag.dropFirst()) : tag
        let assets = release["assets"] as? [[String: Any]] ?? []
        let ipa = assets.first { ($0["name"] as? String ?? "").hasSuffix(".ipa") }?["browser_download_url"] as? String
        if Self.newer(version, than: current), let ipa = ipa.flatMap(URL.init(string:)) {
            available = AppUpdate(version: version, ipa: ipa, page: page)
            dismissed = false
        } else {
            available = nil
            upToDate = !quiet
        }
    }

    /// SideStore installs it; without SideStore, the release page.
    func open() {
        #if os(iOS)
        guard let update = available else { return }
        var link = URLComponents(string: "sidestore://install")!
        link.queryItems = [URLQueryItem(name: "url", value: update.ipa.absoluteString)]
        UIApplication.shared.open(link.url!) { opened in
            if !opened { UIApplication.shared.open(update.page) }
        }
        #endif
    }

    /// "0.10.1" > "0.9.6", part by part.
    nonisolated static func newer(_ candidate: String, than current: String) -> Bool {
        let a = candidate.split(separator: ".").map { Int($0) ?? 0 }
        let b = current.split(separator: ".").map { Int($0) ?? 0 }
        for i in 0..<max(a.count, b.count) {
            let x = i < a.count ? a[i] : 0, y = i < b.count ? b[i] : 0
            if x != y { return x > y }
        }
        return false
    }
}

/// A newer release is out: what it is, and the button that gets it.
struct UpdateBanner: View {
    var body: some View {
        let updater = Updater.shared
        if let update = updater.available, !updater.dismissed {
            HStack(spacing: 10) {
                Image(systemName: "arrow.down.app")
                Text("Version \(update.version) is available")
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                Spacer(minLength: 0)
                Button("Later") { updater.dismissed = true }
                    .foregroundStyle(YT.secondary)
                Button("Update") { updater.open() }
                    .fontWeight(.semibold)
            }
            .font(.footnote)
            .buttonStyle(.plain)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(YT.surface)
        }
    }
}
