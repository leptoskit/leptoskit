import 'dart:typed_data';

import 'pod_error.dart';
import 'reader.dart';
import 'writer.dart';

const podMagic = [0x4c, 0x4b, 0x30, 0x31]; // LK01
const podContentType = 'application/vnd.leptoskit.rpc.v1+bin';
const podCodecVersion = 1;

enum MessageType {
  rpc(1),
  live(2);

  const MessageType(this.raw);
  final int raw;

  static MessageType fromU16(int value) {
    switch (value) {
      case 1:
        return MessageType.rpc;
      case 2:
        return MessageType.live;
      default:
        throw PodInvalidTag(value);
    }
  }
}

class Envelope {
  const Envelope({
    required this.codecVer,
    required this.msgType,
    required this.flags,
    required this.body,
  });

  final int codecVer;
  final MessageType msgType;
  final int flags;
  final Uint8List body;

  @override
  bool operator ==(Object other) =>
      other is Envelope &&
      codecVer == other.codecVer &&
      msgType == other.msgType &&
      flags == other.flags &&
      listEquals(body, other.body);

  @override
  int get hashCode => Object.hash(codecVer, msgType, flags, Object.hashAll(body));
}

bool listEquals<T>(List<T> a, List<T> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

Uint8List wrap(MessageType msgType, int flags, Uint8List body) {
  if (body.length > 0xffffffff) throw const PodTooLarge();
  final w = Writer();
  w.raw(podMagic);
  w.u16(podCodecVersion);
  w.u16(msgType.raw);
  w.u16(flags);
  w.u32(body.length);
  w.raw(body);
  return w.asSlice();
}

Envelope unwrap(Uint8List bytes) {
  if (bytes.length < 14) throw const PodTruncated();
  if (!listEquals(bytes.sublist(0, 4), podMagic)) {
    throw const PodInvalidEnvelope();
  }
  final r = Reader(bytes.sublist(4));
  final codecVer = r.u16();
  if (codecVer != podCodecVersion) throw const PodInvalidEnvelope();
  final msgType = MessageType.fromU16(r.u16());
  final flags = r.u16();
  final bodyLen = r.u32();
  final body = r.remaining;
  if (body.length != bodyLen) throw const PodTruncated();
  return Envelope(
    codecVer: codecVer,
    msgType: msgType,
    flags: flags,
    body: Uint8List.fromList(body),
  );
}
