import SwiftUI
import YTMKit

/// A spinner; after a few seconds it says why it takes so long.
struct LoadingView: View {
    @State private var slow = false

    var body: some View {
        VStack(spacing: 14) {
            ProgressView()
            if !Connectivity.shared.online {
                hint("Waiting for the internet connection…")
            } else if slow {
                hint("Slow connection: still loading…")
            }
        }
        .padding()
        .task {
            try? await Task.sleep(for: .seconds(6))
            slow = true
        }
    }

    private func hint(_ text: LocalizedStringKey) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
    }
}

/// What went wrong and what to do: Retry, and offline a way to the downloads.
struct ErrorView: View {
    let error: Error
    let retry: () -> Void
    @Environment(Library.self) private var library
    @Environment(Router.self) private var router

    var body: some View {
        let problem = error.problem
        ContentUnavailableView {
            Label(title(problem), systemImage: icon(problem))
        } description: {
            Text(detail(problem))
        } actions: {
            Button("Retry", action: retry)
                .buttonStyle(.borderedProminent)
            if problem == .offline, !library.downloads.isEmpty {
                Button("Downloads") { router.openDownloads() }
            }
        }
    }

    private func title(_ problem: Problem) -> String {
        switch problem {
        case .offline: String(localized: "You're offline")
        case .slow: String(localized: "Slow connection")
        case .server: String(localized: "YouTube Music isn't answering")
        case .signIn: String(localized: "Sign in needed")
        case .other: String(localized: "Something went wrong")
        }
    }

    private func detail(_ problem: Problem) -> String {
        switch problem {
        case .offline: String(localized: "Connect to the internet to see this. Downloaded songs still play.")
        case .slow: String(localized: "YouTube Music took too long to answer.")
        case .server: String(localized: "Its servers have a problem right now. Try again in a minute.")
        case .signIn, .other: error.localizedDescription
        }
    }

    private func icon(_ problem: Problem) -> String {
        switch problem {
        case .offline: "wifi.slash"
        case .slow: "tortoise"
        case .server: "exclamationmark.icloud"
        case .signIn: "lock"
        case .other: "exclamationmark.triangle"
        }
    }
}

/// Over every screen: "No internet connection" while offline, "Back
/// online" for a moment when it returns.
struct ConnectionBanner: View {
    @State private var backOnline = false

    var body: some View {
        let online = Connectivity.shared.online
        Group {
            if !online || backOnline {
                HStack(spacing: 8) {
                    Image(systemName: online ? "wifi" : "wifi.slash")
                    Text(online ? "Back online" : "No internet connection · downloaded songs still play")
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                    Spacer(minLength: 0)
                }
                .font(.footnote.weight(.medium))
                .foregroundStyle(.white)
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(online ? Color.green.opacity(0.55) : Color(white: 0.23))
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.25), value: online)
        .animation(.easeInOut(duration: 0.25), value: backOnline)
        .onChange(of: Connectivity.shared.cameBack) {
            backOnline = true
            Task {
                try? await Task.sleep(for: .seconds(2.5))
                backOnline = false
            }
        }
    }
}

/// The bottom message (`Notice`), as a capsule.
struct NoticeToast: View {
    var body: some View {
        if let text = Notice.shared.text {
            Text(text)
                .font(.footnote.weight(.medium))
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .background(.regularMaterial, in: Capsule())
                .padding(.bottom, 90)
                .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }
}
