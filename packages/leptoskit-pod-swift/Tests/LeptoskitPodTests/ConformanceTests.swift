import XCTest
@testable import LeptoskitPod

final class ConformanceTests: XCTestCase {
	private struct Vector: Decodable {
		let name: String
		let hex: String
		let envelope: Bool?
	}

	private struct Malformed: Decodable {
		let name: String
		let hex: String
		let expect: String
		let envelope: Bool?
	}

	private struct Corpus: Decodable {
		let vectors: [Vector]
		let malformed: [Malformed]
	}

	private func hexDecode(_ hex: String) -> [UInt8] {
		var out: [UInt8] = []
		var i = hex.startIndex
		while i < hex.endIndex {
			let j = hex.index(i, offsetBy: 2)
			out.append(UInt8(hex[i..<j], radix: 16)!)
			i = j
		}
		return out
	}

	private func hexEncode(_ bytes: [UInt8]) -> String {
		bytes.map { String(format: "%02x", $0) }.joined()
	}

	private func loadCorpus() throws -> Corpus {
		let url = URL(fileURLWithPath: #filePath)
			.deletingLastPathComponent()
			.deletingLastPathComponent()
			.deletingLastPathComponent()
			.deletingLastPathComponent()
			.appendingPathComponent("crates/leptoskit_pod/conformance/vectors.json")
		let data = try Data(contentsOf: url)
		return try JSONDecoder().decode(Corpus.self, from: data)
	}

	func testConformanceVectors() throws {
		let corpus = try loadCorpus()
		for v in corpus.vectors {
			let bytes = hexDecode(v.hex)
			if v.envelope == true {
				_ = try unwrap(bytes)
			}
		}
	}

	func testMalformedVectors() throws {
		let corpus = try loadCorpus()
		for m in corpus.malformed {
			let bytes = hexDecode(m.hex)
			let err: PodError
			if m.envelope == true {
				do {
					_ = try unwrap(bytes)
					XCTFail("\(m.name): expected error")
					continue
				} catch let e as PodError {
					err = e
				}
			} else if m.name.contains("utf8") {
				var r = Reader(input: bytes)
				do {
					_ = try r.string()
					XCTFail("\(m.name): expected error")
					continue
				} catch let e as PodError {
					err = e
				}
			} else if m.name.contains("bool") {
				var r = Reader(input: bytes)
				do {
					_ = try r.bool()
					XCTFail("\(m.name): expected error")
					continue
				} catch let e as PodError {
					err = e
				}
			} else {
				var r = Reader(input: bytes)
				do {
					_ = try r.u16()
					XCTFail("\(m.name): expected error")
					continue
				} catch let e as PodError {
					err = e
				}
			}
			XCTAssertTrue(
				err.debugDescription.contains(m.expect),
				"\(m.name): expected \(m.expect), got \(err.debugDescription)"
			)
		}
	}

	func testEncodeNoteMatchesVector() throws {
		var w = Writer()
		w.structHeader(version: 0)
		w.u8(7)
		try w.bool(true)
		try w.string("hi")
		XCTAssertEqual(hexEncode(w.asSlice()), "00000701020000006869")
	}

	func testEnvelopeRpcEmpty() throws {
		let framed = try wrap(msgType: .rpc, flags: 0, body: [])
		XCTAssertEqual(hexEncode(framed), "4c4b303101000100000000000000")
	}

	func testTrailingAndTruncation() {
		var r = Reader(input: [1, 2])
		XCTAssertEqual(try? r.u8(), 1)
		XCTAssertThrowsError(try r.finish()) { error in
			XCTAssertEqual(error as? PodError, .trailing)
		}
		var r2 = Reader(input: [1])
		XCTAssertThrowsError(try r2.u16()) { error in
			XCTAssertEqual(error as? PodError, .truncated)
		}
	}
}
