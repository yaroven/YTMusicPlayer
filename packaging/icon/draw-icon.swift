// Draws the app icon (a tilted red tile with a white note on a dark,
// red-glowing background) for every platform. Run by make-icons.sh.
//   draw-icon <out dir>
import AppKit
import CoreGraphics

let N: CGFloat = 1000  // drawing units, y down

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat(hex >> 16 & 0xff) / 255, green: CGFloat(hex >> 8 & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: a)
}

func rounded(_ r: CGRect, _ radius: CGFloat) -> CGPath {
    CGPath(roundedRect: r, cornerWidth: radius, cornerHeight: radius, transform: nil)
}

/// Two beamed eighth notes, white.
func note(_ c: CGContext, scale s: CGFloat) {
    c.saveGState()
    c.translateBy(x: 500, y: 505)
    c.scaleBy(x: s, y: s)
    c.translateBy(x: -500, y: -500)
    c.setFillColor(rgb(0xffffff))
    c.setStrokeColor(rgb(0xffffff))
    c.addPath(rounded(CGRect(x: 345, y: 330, width: 62, height: 390), 31)); c.fillPath()
    c.addPath(rounded(CGRect(x: 625, y: 270, width: 62, height: 380), 31)); c.fillPath()
    let beam = CGMutablePath()
    beam.move(to: CGPoint(x: 345, y: 300)); beam.addLine(to: CGPoint(x: 687, y: 238))
    beam.addLine(to: CGPoint(x: 687, y: 345)); beam.addLine(to: CGPoint(x: 345, y: 407)); beam.closeSubpath()
    c.setLineJoin(.round); c.setLineWidth(36)
    c.addPath(beam); c.drawPath(using: .fillStroke)
    c.fillEllipse(in: CGRect(x: 225, y: 615, width: 182, height: 170))
    c.fillEllipse(in: CGRect(x: 505, y: 545, width: 182, height: 170))
    c.restoreGState()
}

/// The dark background with the red glow.
func background(_ c: CGContext) {
    c.setFillColor(rgb(0x121212)); c.fill(CGRect(x: 0, y: 0, width: N, height: N))
    glow(c)
}

func glow(_ c: CGContext) {
    let glow = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(),
                          colors: [rgb(0xff3d52, 0.55), rgb(0xff3d52, 0)] as CFArray, locations: [0, 1])!
    c.drawRadialGradient(glow, startCenter: CGPoint(x: 500, y: 520), startRadius: 0,
                         endCenter: CGPoint(x: 500, y: 520), endRadius: 520, options: [])
}

/// The tilted tile with its gloss and the note.
func tile(_ c: CGContext) {
    c.saveGState()
    c.translateBy(x: 500, y: 500); c.rotate(by: -0.12); c.translateBy(x: -500, y: -500)
    c.addPath(rounded(CGRect(x: 190, y: 190, width: 620, height: 620), 170)); c.clip()
    let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(),
                          colors: [rgb(0xff6a5c), rgb(0xff2d55), rgb(0xd1104a)] as CFArray, locations: [0, 0.5, 1])!
    c.drawLinearGradient(grad, start: CGPoint(x: 190, y: 190), end: CGPoint(x: 810, y: 810), options: [])
    c.setFillColor(rgb(0xffffff, 0.16)); c.fillEllipse(in: CGRect(x: 60, y: -120, width: 900, height: 520))
    c.restoreGState()
    note(c, scale: 0.62)
}

enum Shape {
    /// Edge to edge, opaque (iPhone; the system rounds it).
    case square
    /// Rounded corners, filling the canvas (Windows, Linux, old Android).
    case roundedFull
    /// macOS: the rounded body inside Apple's grid margin, with a shadow.
    case mac
    /// Android adaptive foreground: no background, inside the safe zone.
    case adaptiveForeground
}

func render(_ shape: Shape, size: Int) -> CGImage {
    let c = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                      space: CGColorSpaceCreateDeviceRGB(),
                      bitmapInfo: (shape == .square ? CGImageAlphaInfo.noneSkipLast : .premultipliedLast).rawValue)!
    c.interpolationQuality = .high
    c.translateBy(x: 0, y: CGFloat(size)); c.scaleBy(x: CGFloat(size) / N, y: -CGFloat(size) / N)
    switch shape {
    case .square:
        background(c); tile(c)
    case .roundedFull:
        c.addPath(rounded(CGRect(x: 20, y: 20, width: 960, height: 960), 215)); c.clip()
        c.translateBy(x: 20, y: 20); c.scaleBy(x: 0.96, y: 0.96)
        background(c); tile(c)
    case .mac:
        // Apple's grid: an 824 body in 1024, 185 corner radius, soft shadow.
        let inset: CGFloat = 100 / 1024 * N, body = N - 2 * inset
        c.saveGState()
        c.setShadow(offset: CGSize(width: 0, height: 12), blur: 28, color: rgb(0x000000, 0.45))
        c.addPath(rounded(CGRect(x: inset, y: inset, width: body, height: body), body * 0.225))
        c.setFillColor(rgb(0x121212)); c.fillPath()
        c.restoreGState()
        c.addPath(rounded(CGRect(x: inset, y: inset, width: body, height: body), body * 0.225)); c.clip()
        c.translateBy(x: inset, y: inset); c.scaleBy(x: body / N, y: body / N)
        background(c); tile(c)
    case .adaptiveForeground:
        // 108 dp layer, the launcher shows the middle 72 and masks it.
        c.translateBy(x: 500, y: 500); c.scaleBy(x: 0.72, y: 0.72); c.translateBy(x: -500, y: -500)
        glow(c); tile(c)
    }
    return c.makeImage()!
}

func save(_ image: CGImage, _ path: String) {
    let rep = NSBitmapImageRep(cgImage: image)
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

let out = CommandLine.arguments[1]
let fm = FileManager.default
try! fm.createDirectory(atPath: "\(out)/ytm-player.iconset", withIntermediateDirectories: true)
// iPhone
save(render(.square, size: 1024), "\(out)/ios-1024.png")
// macOS .iconset (iconutil makes the .icns)
for (pt, scales) in [(16, [1, 2]), (32, [1, 2]), (128, [1, 2]), (256, [1, 2]), (512, [1, 2])] {
    for s in scales {
        let name = s == 1 ? "icon_\(pt)x\(pt).png" : "icon_\(pt)x\(pt)@2x.png"
        save(render(.mac, size: pt * s), "\(out)/ytm-player.iconset/\(name)")
    }
}
// Windows .ico frames, Linux, the window / tray icon
for size in [16, 24, 32, 48, 64, 128, 256] {
    save(render(.roundedFull, size: size), "\(out)/rounded-\(size).png")
}
// Android: legacy launcher icons per density, adaptive foreground
for (density, px) in [("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)] {
    save(render(.roundedFull, size: px), "\(out)/android-\(density).png")
}
save(render(.adaptiveForeground, size: 432), "\(out)/android-foreground.png")
