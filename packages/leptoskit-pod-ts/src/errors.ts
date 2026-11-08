export type PodErrorCode =
  | "Truncated"
  | "Trailing"
  | "TooLarge"
  | "InvalidUtf8"
  | "InvalidTag"
  | "InvalidEnvelope"
  | "UnknownVersion";

export class PodError extends Error {
  readonly code: PodErrorCode;
  readonly tag?: number;
  readonly version?: number;

  constructor(code: PodErrorCode, message?: string, extra?: { tag?: number; version?: number }) {
    super(message ?? code);
    this.name = code;
    this.code = code;
    this.tag = extra?.tag;
    this.version = extra?.version;
  }

  static truncated(): PodError {
    return new PodError("Truncated", "truncated pod buffer");
  }

  static trailing(): PodError {
    return new PodError("Trailing", "trailing bytes after pod value");
  }

  static tooLarge(): PodError {
    return new PodError("TooLarge", "pod value too large");
  }

  static invalidUtf8(): PodError {
    return new PodError("InvalidUtf8", "invalid utf-8 in pod string");
  }

  static invalidTag(tag: number): PodError {
    return new PodError("InvalidTag", `invalid pod tag ${tag}`, { tag });
  }

  static invalidEnvelope(): PodError {
    return new PodError("InvalidEnvelope", "invalid pod envelope");
  }

  static unknownVersion(version: number): PodError {
    return new PodError("UnknownVersion", `unknown pod struct version ${version}`, {
      version,
    });
  }
}
