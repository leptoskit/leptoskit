import { PodError } from "./errors.js";
import { Reader } from "./reader.js";
import { Writer } from "./writer.js";

export {
	CODEC_VERSION,
	CONTENT_TYPE,
	type Envelope,
	MAGIC,
	MessageType,
	unwrap,
	wrap,
} from "./envelope.js";
export { PodError, type PodErrorCode } from "./errors.js";
export { Reader } from "./reader.js";
export { Writer } from "./writer.js";

export function encodeStruct(
	disc: number,
	version: number,
	encodeFields: (w: Writer) => void,
): Uint8Array {
	const w = Writer.new();
	w.structHeader(disc, version);
	encodeFields(w);
	return w.finish();
}

export function encodeStructV0(
	disc: number,
	encodeFields: (w: Writer) => void,
): Uint8Array {
	return encodeStruct(disc, 0, encodeFields);
}

export function decodeStructVersion<T>(
	r: Reader,
	expectedDisc: number,
	expectedVersion: number,
	decodeFields: (r: Reader) => T,
): T {
	const [disc, version] = r.structHeader();
	if (disc !== expectedDisc) {
		throw PodError.unknownDiscriminator(disc);
	}
	if (version !== expectedVersion) {
		throw PodError.unknownVersion(version);
	}
	return decodeFields(r);
}
