import CLeptosKitBridge
import CoreText
import Foundation
import LeptosKitBridge

/// The app's declared fonts: the faces `leptoskit.toml` declares under
/// `[theme.fonts]`, which the build lists in the bundle's `fonts.json` and
/// bundles in its `assets` folder.
///
/// `install` registers every file with Core Text and hands the manifest to
/// the Rust host before the app is created, so `font-[var(--name)]` resolves
/// to the declared family and layout measures in the face that draws. The
/// renderers take a declared family's font from `font(family:weight:italic:size:)`,
/// which chooses the face exactly as layout did.
@MainActor
enum NativeFonts {
    /// The OpenType tag of the weight axis, `wght`, as Core Text names axes.
    private static let weightAxis = 0x7767_6874

    /// The most fonts kept: an animated arbitrary size makes a new one every
    /// frame, and the cache outlives every app the process runs.
    private static let fontLimit = 64

    private struct Manifest: Decodable { let families: [ManifestFamily] }
    private struct ManifestFamily: Decodable { let faces: [ManifestFace] }
    private struct ManifestFace: Decodable {
        let file: String
        let weight: [UInt32]
    }

    private struct Declared {
        let url: URL
        let variable: Bool
    }

    private struct Key: Hashable {
        let index: Int
        let weight: UInt32
        let size: CGFloat
    }

    private static var installed = false
    private static var declared: [Declared] = []
    private static var fonts: [Key: CTFont] = [:]

    /// Registers and installs the fonts `bundle` declares; later calls change
    /// nothing. A bundle built without `fonts.json` declares no fonts.
    static func install(bundle: Bundle = .main) throws {
        guard !installed else { return }
        installed = true

        guard let manifestURL = bundle.url(forResource: "fonts", withExtension: "json"),
              let resources = bundle.resourceURL
        else { return }

        let manifest = try Data(contentsOf: manifestURL)
        let root = resources.appendingPathComponent("assets", isDirectory: true)
        let path = Array(root.path.utf8)
        let status = manifest.withUnsafeBytes { bytes in
            path.withUnsafeBufferPointer { path in
                lk_fonts_install(
                    bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, path.baseAddress, path.count
                )
            }
        }

        guard status == 0 else {
            throw NativeAppError(message: "The app's fonts could not be installed (status \(status))")
        }

        declared = try JSONDecoder().decode(Manifest.self, from: manifest).families.flatMap { family in
            family.faces.map { face in
                Declared(
                    url: face.file.split(separator: "/").reduce(root) { $0.appendingPathComponent(String($1)) },
                    variable: face.weight.first != face.weight.last
                )
            }
        }

        for face in declared {
            var error: Unmanaged<CFError>?

            // Text views look fonts up by name as well as by object.
            if !CTFontManagerRegisterFontsForURL(face.url as CFURL, .process, &error) {
                let reason = error?.takeRetainedValue()
                NativeLog.error(NativeLog.render, "The font \(face.url.lastPathComponent) was not registered", reason)
            }
        }
    }

    /// The CSS weight, from 100 to 900, of a weight a render transaction names.
    static func cssWeight(_ name: String?) -> UInt32 {
        switch name {
        case "thin": 100
        case "extra_light": 200
        case "light": 300
        case "medium": 500
        case "semibold": 600
        case "bold": 700
        case "extra_bold": 800
        case "black": 900
        default: 400
        }
    }

    /// The font text of declared `family` draws in at `weight` and `italic`,
    /// or nil when it draws the system font: no family, or none installed.
    static func font(family: UInt32?, weight: String?, italic: Bool, size: CGFloat) -> CTFont? {
        guard let family else { return nil }
        var face: [UInt32] = [0, 0]

        guard lk_fonts_select(family, cssWeight(weight), italic, &face) == 1 else { return nil }

        return self.face(index: Int(face[0]), weight: face[1], size: size)
    }

    /// The declared face at its position in `fonts.json`, drawn at `weight`
    /// and `size`, or nil when its file no longer loads.
    static func face(index: Int, weight: UInt32, size: CGFloat) -> CTFont? {
        guard declared.indices.contains(index) else { return nil }
        let face = declared[index]
        let key = Key(index: index, weight: face.variable ? weight : 0, size: size)

        if let cached = fonts[key] { return cached }

        // The build checks every declared file, so one that does not load
        // means the bundle was changed after it was built.
        guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(face.url as CFURL) as? [CTFontDescriptor],
              var descriptor = descriptors.first
        else {
            NativeLog.error(NativeLog.render, "The font \(face.url.lastPathComponent) could not be loaded")
            return nil
        }

        if face.variable {
            descriptor = CTFontDescriptorCreateCopyWithVariation(
                descriptor, NSNumber(value: weightAxis) as CFNumber, CGFloat(weight)
            )
        }

        let font = CTFontCreateWithFontDescriptor(descriptor, size, nil)

        if fonts.count >= fontLimit { fonts.removeAll(keepingCapacity: true) }

        fonts[key] = font

        return font
    }
}
