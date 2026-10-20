import SwiftUI
#if canImport(WebKit)
import WebKit
#endif

/// The song's clip, in place of the cover — what YouTube Music's song /
/// video switch does. A web view shows YouTube's watch page, stripped down
/// to the player by `pageScript` (the same script as the desktop player's
/// `video::PAGE_SCRIPT`); the page reports where the video is, so the audio
/// continues there when you switch back. The embed player would be lighter,
/// but YouTube refuses to embed many songs ("error 153").
struct VideoPane: View {
    let videoId: String
    let start: TimeInterval
    /// Called with every position the page reports.
    let progress: (TimeInterval) -> Void

    var body: some View {
        #if os(iOS)
        VideoWebView(videoId: videoId, start: start, progress: progress)
            .aspectRatio(16 / 9, contentMode: .fit)
            .background(Color.black)
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        #else
        Color.black
        #endif
    }
}

/// YouTube Music's ♪ / ▶ switch over the cover.
struct MediaSwitch: View {
    @Binding var video: Bool
    /// The song has a clip; without one "Video" is dimmed and does nothing.
    var hasVideo = true

    var body: some View {
        HStack(spacing: 4) {
            button(false, "headphones")
            button(true, "play.rectangle")
        }
        .padding(4)
        .background(.white.opacity(0.1), in: Capsule())
    }

    private func button(_ value: Bool, _ icon: String) -> some View {
        Button { video = value } label: {
            Image(systemName: icon)
                .font(.body.weight(.semibold))
                .foregroundStyle(video == value ? Color.primary : Color.secondary)
                .frame(width: 44, height: 30)
                .background(video == value ? Color.white.opacity(0.15) : .clear, in: Capsule())
        }
        .buttonStyle(.plain)
        .disabled(value && !hasVideo)
        .opacity(value && !hasVideo ? 0.35 : 1)
        .accessibilityLabel(value ? "Video" : "Song")
    }
}

/// Dark theme, nothing but the player, the player over the whole view, and
/// the position posted every half second.
let pageScript = """
(function () {
    window.ipc = { postMessage: function (m) { window.webkit.messageHandlers.ipc.postMessage(m); } };
    document.documentElement.setAttribute('dark', 'true');
    var css = document.createElement('style');
    css.textContent =
        'html,body{background:#000!important;margin:0!important;overflow:hidden!important}'
        + 'ytd-masthead,#masthead-container,#secondary,#below,ytd-comments,#chat,'
        + 'tp-yt-app-drawer,#related,ytd-watch-metadata,#panels,ytd-popup-container'
        + '{display:none!important}'
        + 'ytd-app,#content,#page-manager,ytd-watch-flexy,#columns,#primary,#primary-inner,'
        + '#player,#player-container-outer,#player-container-inner,#movie_player'
        + '{background:#000!important}'
        + 'ytd-watch-flexy #primary,#player-container-outer,#player-container-inner'
        + '{padding:0!important;margin:0!important;max-width:100vw!important}'
        + '#player,#movie_player,.html5-video-player,.html5-video-container'
        + '{width:100vw!important;height:100vh!important;left:0!important;top:0!important}'
        + 'video{width:100%!important;height:100%!important;left:0!important;top:0!important;'
        + 'object-fit:contain!important}';
    (document.head || document.documentElement).appendChild(css);
    setInterval(function () {
        var v = document.querySelector('video');
        if (v) { window.ipc.postMessage(String(v.currentTime)); }
    }, 500);
})();
"""

#if os(iOS)
private struct VideoWebView: UIViewRepresentable {
    let videoId: String
    let start: TimeInterval
    let progress: (TimeInterval) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(progress: progress) }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []
        config.userContentController.addUserScript(
            WKUserScript(source: pageScript, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController.add(context.coordinator, name: "ipc")
        let view = WKWebView(frame: .zero, configuration: config)
        view.isOpaque = false
        view.backgroundColor = .black
        view.scrollView.isScrollEnabled = false
        // The desktop page: the mobile one (m.youtube.com) is laid out
        // differently and the script wouldn't strip it.
        view.customUserAgent =
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15"
        let url = URL(string: "https://www.youtube.com/watch?v=\(videoId)&t=\(Int(start))s")!
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        view.configuration.userContentController.removeScriptMessageHandler(forName: "ipc")
        view.stopLoading()
    }

    final class Coordinator: NSObject, WKScriptMessageHandler {
        let progress: (TimeInterval) -> Void
        init(progress: @escaping (TimeInterval) -> Void) { self.progress = progress }

        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
            if let text = message.body as? String, let seconds = TimeInterval(text), seconds.isFinite {
                progress(seconds)
            }
        }
    }
}
#endif
