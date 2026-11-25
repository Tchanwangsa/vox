// Draws Vox's icon — the pill's waveform on its dark ground — and writes
// the sizes an .icns needs into the iconset directory given as argument 1.
//
//   swift scripts/icon.swift assets/Vox.iconset && iconutil -c icns assets/Vox.iconset
import AppKit

let out = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

func draw(_ side: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(side)
    // The macOS icon grid: the shape fills 824 of 1024 points.
    let inset = s * 100 / 1024
    let tile = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let shape = NSBezierPath(roundedRect: tile, xRadius: tile.width * 0.225, yRadius: tile.width * 0.225)
    NSGradient(
        starting: NSColor(srgbRed: 0.14, green: 0.16, blue: 0.23, alpha: 1),
        ending: NSColor(srgbRed: 0.06, green: 0.07, blue: 0.11, alpha: 1))!
        .draw(in: shape, angle: -90)

    let heights: [CGFloat] = [0.16, 0.34, 0.62, 0.40, 0.86, 0.52, 0.28, 0.14]
    let bar = tile.width * 0.058
    let gap = tile.width * 0.040
    let dot = tile.width * 0.075
    let total = dot + gap * 1.6 + CGFloat(heights.count) * bar + CGFloat(heights.count - 1) * gap
    var x = tile.midX - total / 2

    NSColor(srgbRed: 1.0, green: 0.42, blue: 0.34, alpha: 1).setFill()
    NSBezierPath(ovalIn: NSRect(x: x, y: tile.midY - dot / 2, width: dot, height: dot)).fill()
    x += dot + gap * 1.6

    for (i, h) in heights.enumerated() {
        let height = tile.height * 0.5 * h + bar
        NSColor(white: 1, alpha: 0.45 + 0.55 * CGFloat(i + 1) / CGFloat(heights.count)).setFill()
        NSBezierPath(
            roundedRect: NSRect(x: x, y: tile.midY - height / 2, width: bar, height: height),
            xRadius: bar / 2, yRadius: bar / 2
        ).fill()
        x += bar + gap
    }
    NSGraphicsContext.current = nil
    return rep.representation(using: .png, properties: [:])!
}

for (points, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
    let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
    try draw(points * scale).write(to: out.appendingPathComponent(name))
}
