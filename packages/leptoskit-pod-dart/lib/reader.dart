import 'dart:convert';
import 'dart:typed_data';

import 'pod_error.dart';

/// Allocation-free Pod decoder over a borrowed buffer.
class Reader {
  Reader(Uint8List input) : _remaining = Uint8List.fromList(input);

  Uint8List _remaining;

  Uint8List get remaining => _remaining;

  Uint8List _take(int len) {
    if (_remaining.length < len) throw const PodTruncated();
    final slice = _remaining.sublist(0, len);
    _remaining = _remaining.sublist(len);
    return slice;
  }

  int u8() => _take(1)[0];

  int u16() {
    final b = _take(2).buffer.asByteData();
    return b.getUint16(0, Endian.little);
  }

  int u32() {
    final b = _take(4).buffer.asByteData();
    return b.getUint32(0, Endian.little);
  }

  int u64() {
    final b = _take(8).buffer.asByteData();
    return b.getUint64(0, Endian.little);
  }

  int i8() {
    final v = u8();
    return v >= 128 ? v - 256 : v;
  }

  int i16() {
    final v = u16();
    return v >= 0x8000 ? v - 0x10000 : v;
  }

  int i32() {
    final v = u32();
    return v >= 0x80000000 ? v - 0x100000000 : v;
  }

  int i64() => u64().toSigned(64);

  bool podBool() {
    switch (u8()) {
      case 0:
        return false;
      case 1:
        return true;
      case final other:
        throw PodInvalidTag(other);
    }
  }

  Uint8List bytes() {
    final len = u32();
    return _take(len);
  }

  String string() {
    final slice = bytes();
    try {
      return utf8.decode(slice);
    } on FormatException {
      throw const PodInvalidUtf8();
    }
  }

  int vecLen() => u32();

  bool optionTag() {
    switch (u8()) {
      case 0:
        return false;
      case 1:
        return true;
      case final other:
        throw PodInvalidTag(other);
    }
  }

  bool resultTag() {
    switch (u8()) {
      case 0:
        return true;
      case 1:
        return false;
      case final other:
        throw PodInvalidTag(other);
    }
  }

  int structHeader() => u16();

  int variant() => u16();

  void finish() {
    if (_remaining.isNotEmpty) throw const PodTrailing();
  }
}
