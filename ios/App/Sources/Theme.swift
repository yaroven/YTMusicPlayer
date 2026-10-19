import SwiftUI
import YTMKit

/// YouTube Music's look: near-black, white text, grey secondary text,
/// translucent chips, red only for the logo and the progress.
enum YT {
    static let background = Color(red: 0.012, green: 0.012, blue: 0.012)
    static let surface = Color(red: 0.13, green: 0.13, blue: 0.13)
    static let raised = Color.white.opacity(0.1)
    static let secondary = Color(white: 0.67)
    static let red = Color(red: 1, green: 0, blue: 0)
    static let sectionTitle = Font.system(size: 22, weight: .bold)
}

/// The "▶ Music" logo.
struct YTLogo: View {
    var body: some View {
        HStack(spacing: 4) {
            ZStack {
                Circle().fill(YT.red)
                Circle().stroke(.white, lineWidth: 1.6).padding(5)
                Image(systemName: "play.fill")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.white)
                    .offset(x: 1)
            }
            .frame(width: 28, height: 28)
            Text("Music")
                .font(.system(size: 22, weight: .bold))
                .tracking(-0.6)
        }
        .accessibilityLabel("YouTube Music")
    }
}

/// The account's picture (or a person), which opens Settings.
struct AvatarButton: View {
    @Environment(Account.self) private var account
    @State private var settings = false

    var body: some View {
        Button { settings = true } label: {
            Group {
                if let photo = account.photo {
                    AsyncImage(url: photo) { $0.resizable().scaledToFill() } placeholder: { YT.raised }
                } else {
                    Image(systemName: account.signedIn ? "person.crop.circle.fill" : "person.crop.circle")
                        .resizable()
                        .foregroundStyle(YT.secondary)
                }
            }
            .frame(width: 32, height: 32)
            .clipShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Settings")
        .sheet(isPresented: $settings) { SettingsSheet() }
    }
}

/// A rounded chip: Home's moods, search kinds, library filters.
struct ChipButton: View {
    let title: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(selected ? .black : .white)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(selected ? Color.white : YT.raised, in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
    }
}

/// "Play all" and friends: an outlined pill.
struct OutlinePill: View {
    let title: LocalizedStringKey
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 14, weight: .medium))
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .overlay(Capsule().stroke(.white.opacity(0.3)))
        }
        .buttonStyle(.plain)
    }
}

/// A round icon button with a translucent fill (page and player actions).
struct CircleIcon: View {
    let icon: String
    var size: CGFloat = 44
    var filled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: size * 0.42, weight: .semibold))
                .foregroundStyle(filled ? .black : .white)
                .frame(width: size, height: size)
                .background(filled ? Color.white : YT.raised, in: Circle())
        }
        .buttonStyle(.plain)
    }
}

/// The colours of a picture, blurred, fading into the background at the
/// top of a screen (YouTube Music's ambient header).
struct AmbientBackdrop: View {
    let art: ArtworkSource
    var height: CGFloat = 420

    var body: some View {
        ZStack(alignment: .top) {
            YT.background
            Artwork(source: art, size: 120)
                .scaleEffect(5)
                .blur(radius: 70)
                .opacity(0.55)
                .frame(maxWidth: .infinity)
                .frame(height: height)
                .clipped()
            LinearGradient(colors: [.clear, YT.background], startPoint: .center, endPoint: .bottom)
                .frame(height: height)
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
    }
}

extension View {
    /// Screens with their own header (Home, Search, Library).
    func hiddenNavigationBar() -> some View {
        #if os(iOS)
        toolbar(.hidden, for: .navigationBar)
        #else
        self
        #endif
    }

    /// The bar stays (back button) but lets the screen's colours through.
    func clearNavigationBar() -> some View {
        #if os(iOS)
        toolbarBackground(.hidden, for: .navigationBar)
        #else
        self
        #endif
    }
}
