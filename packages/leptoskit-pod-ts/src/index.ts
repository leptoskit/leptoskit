import { PodError } from "./errors.js";
import { Reader } from "./reader.js";
import { Writer } from "./writer.js";

export { PodError, type PodErrorCode } from "./errors.js";
export { Writer } from "./writer.js";
export { Reader } from "./reader.js";
export {
  wrap,
  unwrap,
  MAGIC,
  CONTENT_TYPE,
  CODEC_VERSION,
  MessageType,
  type Envelope,
} from "./envelope.js";

export function encodeStructV0(encodeFields: (w: Writer) => void): Uint8Array {
  const w = Writer.new();
  w.structHeader(0, 0);
  encodeFields(w);
  return w.finish();
}

export function decodeStructVersion<T>(
  r: Reader,
  expectedVersion: number,
  decodeFields: (r: Reader) => T
): T {
  const [version, flags] = r.structHeader();
  if (version !== expectedVersion) {
    throw PodError.unknownVersion(version);
  }
  if (flags !== 0) {
    throw PodError.invalidTag(flags);
  }
  return decodeFields(r);
}
