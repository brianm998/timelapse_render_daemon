// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors
//
// Draws the shared icon of the Timelapse Render Daemon (TRD-24) offscreen with CoreGraphics. It opens
// no window and starts no NSApplication, so nothing appears on screen.
//
// Usage: swift make_icon.swift --variant trd|monitor --out DIR
//
// Writes into DIR:
//   <variant>-1024.png   the master image
//   <variant>-512.png    512 px (monitor-512.png is this file for the monitor variant)
//   <variant>.icns       made by /usr/bin/iconutil from the ten iconset sizes
//   <variant>-16.png, <variant>-32.png, <variant>-48.png, <variant>-256.png
//                        the sources make_ico.py packs into monitor.ico
// Every size is drawn from the same vector description rather than resampled, so the small sizes
// stay sharp. The intermediate .iconset directory is removed afterwards.
//
// Regenerating the committed files (from this directory, with OUT a temporary directory):
//   swift make_icon.swift --variant trd --out "$OUT"
//   cp "$OUT/trd-1024.png" "$OUT/trd.icns" .
//   swift make_icon.swift --variant monitor --out "$OUT"
//   cp "$OUT/monitor-1024.png" "$OUT/monitor-512.png" "$OUT/monitor.icns" .
//   python3 -I make_ico.py monitor.ico "$OUT/monitor-16.png" "$OUT/monitor-32.png" \
//       "$OUT/monitor-48.png" "$OUT/monitor-256.png"

import CoreGraphics
import Foundation
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}

// Visual coordinates: a 1024 x 1024 square, y growing downwards.
let design: CGFloat = 1024
let navy = CGColor(red: 0x1E / 255, green: 0x2A / 255, blue: 0x44 / 255, alpha: 1)
let amber = CGColor(red: 0xF2 / 255, green: 0xA3 / 255, blue: 0x3A / 255, alpha: 1)
let white = CGColor(gray: 1, alpha: 1)

func draw(_ ctx: CGContext, pixels: Int, variant: String) {
    let s = CGFloat(pixels)
    ctx.clear(CGRect(x: 0, y: 0, width: s, height: s))
    // Map the design square onto the bitmap, with y pointing down as in the drawing below.
    ctx.translateBy(x: 0, y: s)
    ctx.scaleBy(x: s / design, y: -s / design)

    // Background: a rounded square.
    let background = CGPath(roundedRect: CGRect(x: 0, y: 0, width: design, height: design),
                            cornerWidth: 224, cornerHeight: 224, transform: nil)
    ctx.setFillColor(navy)
    ctx.addPath(background)
    ctx.fillPath()

    // Film strip: a white band with sprocket holes top and bottom and four frames that step from
    // dark to light.
    let strip = CGPath(roundedRect: CGRect(x: 150, y: 392, width: 724, height: 240),
                       cornerWidth: 18, cornerHeight: 18, transform: nil)
    ctx.setFillColor(white)
    ctx.addPath(strip)
    ctx.fillPath()

    ctx.setFillColor(navy)
    var holeX: CGFloat = 178
    while holeX + 36 <= 846 {
        ctx.fill(CGRect(x: holeX, y: 406, width: 36, height: 28))
        ctx.fill(CGRect(x: holeX, y: 590, width: 36, height: 28))
        holeX += 64
    }

    let levels: [CGFloat] = [0.10, 0.36, 0.62, 0.92]
    for (index, level) in levels.enumerated() {
        let frameX = 170 + CGFloat(index) * 173
        ctx.setFillColor(CGColor(gray: level, alpha: 1))
        ctx.fill(CGRect(x: frameX, y: 448, width: 165, height: 126))
    }

    // Progress arc, inside the square, starting at the top and sweeping 72 per cent of a turn.
    ctx.setStrokeColor(amber)
    ctx.setLineWidth(22)
    ctx.setLineCap(.round)
    ctx.addArc(center: CGPoint(x: 512, y: 512), radius: 452,
               startAngle: -CGFloat.pi / 2,
               endAngle: -CGFloat.pi / 2 + 2 * CGFloat.pi * 0.72,
               clockwise: false)
    ctx.strokePath()

    // Monitor variant: a white eye in the lower right.
    if variant == "monitor" {
        let ex: CGFloat = 736
        let ey: CGFloat = 736
        ctx.setStrokeColor(white)
        ctx.setFillColor(white)
        ctx.setLineWidth(24)
        ctx.move(to: CGPoint(x: ex - 110, y: ey))
        ctx.addQuadCurve(to: CGPoint(x: ex + 110, y: ey), control: CGPoint(x: ex, y: ey - 80))
        ctx.addQuadCurve(to: CGPoint(x: ex - 110, y: ey), control: CGPoint(x: ex, y: ey + 80))
        ctx.strokePath()
        ctx.fillEllipse(in: CGRect(x: ex - 34, y: ey - 34, width: 68, height: 68))
    }
}

func render(_ pixels: Int, variant: String) -> CGImage {
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    let ctx = CGContext(data: nil, width: pixels, height: pixels, bitsPerComponent: 8,
                        bytesPerRow: 0, space: space,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    draw(ctx, pixels: pixels, variant: variant)
    guard let image = ctx.makeImage() else { fail("make_icon.swift: could not render \(pixels) px") }
    return image
}

func writePNG(_ image: CGImage, to path: String) {
    let url = URL(fileURLWithPath: path) as CFURL
    guard let destination = CGImageDestinationCreateWithURL(url, "public.png" as CFString, 1, nil) else {
        fail("make_icon.swift: cannot create \(path)")
    }
    CGImageDestinationAddImage(destination, image, nil)
    if !CGImageDestinationFinalize(destination) { fail("make_icon.swift: cannot write \(path)") }
}

func run(_ tool: String, _ arguments: [String]) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: tool)
    process.arguments = arguments
    do { try process.run() } catch { fail("make_icon.swift: cannot start \(tool): \(error)") }
    process.waitUntilExit()
    if process.terminationStatus != 0 {
        fail("make_icon.swift: \(tool) \(arguments.joined(separator: " ")) failed with \(process.terminationStatus)")
    }
}

// Parse --variant and --out.
var variant: String?
var outDir: String?
var arguments = CommandLine.arguments.dropFirst().makeIterator()
while let argument = arguments.next() {
    switch argument {
    case "--variant": variant = arguments.next()
    case "--out": outDir = arguments.next()
    default: fail("make_icon.swift: unknown argument \(argument)")
    }
}
guard let chosen = variant, chosen == "trd" || chosen == "monitor", let out = outDir else {
    fail("usage: swift make_icon.swift --variant trd|monitor --out DIR")
}

let fileManager = FileManager.default
try? fileManager.createDirectory(atPath: out, withIntermediateDirectories: true)

writePNG(render(1024, variant: chosen), to: "\(out)/\(chosen)-1024.png")
writePNG(render(512, variant: chosen), to: "\(out)/\(chosen)-512.png")
for size in [16, 32, 48, 256] {
    writePNG(render(size, variant: chosen), to: "\(out)/\(chosen)-\(size).png")
}

// The iconset holds every size iconutil expects: name, then pixels.
let iconset = "\(out)/.\(chosen).iconset"
try? fileManager.removeItem(atPath: iconset)
try! fileManager.createDirectory(atPath: iconset, withIntermediateDirectories: true)
let iconsetSizes: [(String, Int)] = [
    ("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024),
]
for (name, pixels) in iconsetSizes {
    writePNG(render(pixels, variant: chosen), to: "\(iconset)/\(name)")
}
run("/usr/bin/iconutil", ["-c", "icns", iconset, "-o", "\(out)/\(chosen).icns"])
try? fileManager.removeItem(atPath: iconset)
print("make_icon.swift: wrote \(chosen) files to \(out)")
