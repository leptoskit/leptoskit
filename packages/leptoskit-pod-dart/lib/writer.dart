import 'dart:convert';
import 'dart:typed_data';

import 'pod_error.dart';

/// Appends little-endian Pod fields into a growable buffer.
class Writer {
  final BytesBuilder _bytes = BytesBuilder(copy: false);

  int get length => _bytes.length;

  bool get isEmpty => _bytes.isEmpty;

  void raw(List<int> value) => _bytes.add(value);

  void u8(int value) => _bytes.addByte(value & 0xff);

  void u16(int value) {
    final b = ByteData(2)..setUint16(0, value, Endian.little);
    _bytes.add(b.buffer.asUint8List());
  }

  void u32(int value) {
    final b = ByteData(4)..setUint32(0, value, Endian.little);
    _bytes.add(b.buffer.asUint8List());
  }

  void u64(int value) {
    final b = ByteData(8)..setUint64(0, value, Endian.little);
    _bytes.add(b.buffer.asUint8List());
  }

  void i8(int value) => u8(value & 0xff);
  void i16(int value) => u16(value & 0xffff);
  void i32(int value) => u32(value);
  void i64(int value) => u64(value);

  void podBool(bool value) => u8(value ? 1 : 0);

  void bytes(List<int> value) {
    if (value.length > 0xffffffff) throw const PodTooLarge();
    u32(value.length);
    _bytes.add(value);
  }

  void string(String value) => bytes(utf8.encode(value));

  void vecLen(int len) {
    if (len > 0xffffffff) throw const PodTooLarge();
    u32(len);
  }

  void optionTag(bool some) => u8(some ? 1 : 0);

  void resultTag({required bool ok}) => u8(ok ? 0 : 1);

  void structHeader(int version, int flags) {
    u8(version);
    u8(flags);
  }

  void variant(int disc) => u16(disc);

  Uint8List asSlice() => _bytes.toBytes();

  Uint8List finish() {
    final out = asSlice();
    _bytes.clear();
    return out;
  }
}
