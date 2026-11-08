import { PodError } from "./errors.js";
import { Reader } from "./reader.js";
import { Writer } from "./writer.js";

export const MAGIC = new Uint8Array([0x4c, 0x4b, 0x30, 0x31]); // LK01
export const CONTENT_TYPE = "application/vnd.leptoskit.rpc.v1+bin";
export const CODEC_VERSION = 1;

export enum MessageType {
  Rpc = 1,
  Live = 2,
}

export interface Envelope {
  codecVer: number;
  msgType: MessageType;
  flags: number;
  body: Uint8Array;
}

function messageTypeFromU16(value: number): MessageType {
  switch (value) {
    case 1:
      return MessageType.Rpc;
    case 2:
      return MessageType.Live;
    default:
      throw PodError.invalidTag(value);
  }
}

export function wrap(msgType: MessageType, flags: number, body: Uint8Array): Uint8Array {
  if (body.length > 0xffffffff) {
    throw PodError.tooLarge();
  }
  const w = Writer.withCapacity(14 + body.length);
  w.raw(MAGIC);
  w.u16(CODEC_VERSION);
  w.u16(msgType);
  w.u16(flags);
  w.u32(body.length);
  w.raw(body);
  return w.finish();
}

export function unwrap(bytes: Uint8Array): Envelope {
  if (bytes.length < 14) {
    throw PodError.truncated();
  }
  for (let i = 0; i < 4; i++) {
    if (bytes[i] !== MAGIC[i]) {
      throw PodError.invalidEnvelope();
    }
  }
  const r = new Reader(bytes.subarray(4));
  const codecVer = r.u16();
  if (codecVer !== CODEC_VERSION) {
    throw PodError.invalidEnvelope();
  }
  const msgType = messageTypeFromU16(r.u16());
  const flags = r.u16();
  const bodyLen = r.u32();
  const body = r.getRemaining();
  if (body.length !== bodyLen) {
    throw PodError.truncated();
  }
  return { codecVer, msgType, flags, body };
}
