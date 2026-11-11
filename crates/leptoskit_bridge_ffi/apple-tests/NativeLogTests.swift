import Foundation
import LeptosKitBridge
import Testing

private enum PluginFailure: Error {
    case refused
}

@Test
func aFailureRecordNamesEveryErrorUnderIt() {
    let disk = NSError(domain: NSPOSIXErrorDomain, code: 28, userInfo: [
        NSLocalizedDescriptionKey: "No space left on device"
    ])
    let write = NSError(domain: NSCocoaErrorDomain, code: 640, userInfo: [
        NSLocalizedDescriptionKey: "The file could not be saved",
        NSUnderlyingErrorKey: disk
    ])
    let record = NativeLog.describe("cannot start the app", write)
    let lines = record.split(separator: "\n").map(String.init)

    #expect(lines.count == 2)
    #expect(lines[0].hasPrefix("cannot start the app: "))
    #expect(lines[0].contains("NSCocoaErrorDomain 640"))
    #expect(lines[0].contains("The file could not be saved"))
    #expect(lines[1].hasPrefix("caused by "))
    #expect(lines[1].contains("NSPOSIXErrorDomain 28"))
    #expect(lines[1].contains("No space left on device"))
}

@Test
func aSwiftErrorRecordNamesItsType() {
    let record = NativeLog.describe("call #7 threw", PluginFailure.refused)

    #expect(record.hasPrefix("call #7 threw: "))
    #expect(record.contains("PluginFailure"))
}

@Test
func aRecordWithoutAnErrorIsTheMessage() {
    #expect(NativeLog.describe("resumed", nil) == "resumed")
}

@Test
func everyLevelWritesThroughTheRuntime() {
    // Written through `lk_log_write` to the unified log; this checks the
    // FFI call links and accepts each level without crashing.
    NativeLog.info(NativeLog.bridge, "NativeLogTests: info")
    NativeLog.warn(NativeLog.gpu, "NativeLogTests: warn", PluginFailure.refused)
    NativeLog.error(NativeLog.render, "NativeLogTests: error", PluginFailure.refused)
}
