import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:leptoskit_pod/leptoskit_pod.dart';
import 'package:test/test.dart';

void main() {
  final vectorsFile = _vectorsPath();

  group('conformance vectors', () {
    late Map<String, dynamic> corpus;

    setUpAll(() {
      corpus = jsonDecode(vectorsFile.readAsStringSync()) as Map<String, dynamic>;
    });

    test('vectors decode', () {
      for (final v in corpus['vectors'] as List) {
        final bytes = _hexDecode(v['hex'] as String);
        if (v['envelope'] == true) {
          unwrap(Uint8List.fromList(bytes));
        }
      }
    });

    test('malformed', () {
      for (final m in corpus['malformed'] as List) {
        final bytes = _hexDecode(m['hex'] as String);
        final expected = m['expect'] as String;
        final name = m['name'] as String;
        Object? err;
        try {
          final r = Reader(Uint8List.fromList(bytes));
          if (m['envelope'] == true) {
            unwrap(Uint8List.fromList(bytes));
          } else if (name.contains('utf8')) {
            r.string();
          } else if (name.contains('option')) {
            r.optionTag();
          } else if (name.contains('result')) {
            r.resultTag();
          } else if (name.contains('bool')) {
            r.podBool();
          } else if (name.contains('huge_len') || name.contains('vec_u8')) {
            r.bytes();
          } else {
            r.u16();
          }
        } catch (e) {
          err = e;
        }
        expect(err, isNotNull, reason: name);
        expect(err.toString(), contains(expected), reason: name);
      }
    });
  });

  test('encode note matches vector', () {
    final w = Writer();
    w.structHeader(0);
    w.u8(7);
    w.podBool(true);
    w.string('hi');
    expect(_hexEncode(w.asSlice()), '00000701020000006869');
  });

  test('envelope rpc empty', () {
    final framed = wrap(MessageType.rpc, 0, Uint8List(0));
    expect(_hexEncode(framed), '4c4b303101000100000000000000');
  });

  test('trailing and truncation', () {
    final r = Reader(Uint8List.fromList([1, 2]));
    r.u8();
    expect(() => r.finish(), throwsA(isA<PodTrailing>()));

    final r2 = Reader(Uint8List.fromList([1]));
    expect(() => r2.u16(), throwsA(isA<PodTruncated>()));
  });
}

File _vectorsPath() {
  const candidates = [
    'crates/leptoskit_pod/conformance/vectors.json',
    '../../crates/leptoskit_pod/conformance/vectors.json',
  ];
  for (final p in candidates) {
    final f = File(p);
    if (f.existsSync()) return f;
  }
  throw StateError('vectors.json not found');
}

List<int> _hexDecode(String hex) =>
    [for (var i = 0; i < hex.length; i += 2) int.parse(hex.substring(i, i + 2), radix: 16)];

String _hexEncode(Uint8List bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
