/// Errors from encoding or decoding LK Pod bytes.
sealed class PodError implements Exception {
  const PodError();
}

class PodTruncated extends PodError {
  const PodTruncated();
  @override
  String toString() => 'Truncated';
}

class PodTrailing extends PodError {
  const PodTrailing();
  @override
  String toString() => 'Trailing';
}

class PodTooLarge extends PodError {
  const PodTooLarge();
  @override
  String toString() => 'TooLarge';
}

class PodInvalidUtf8 extends PodError {
  const PodInvalidUtf8();
  @override
  String toString() => 'InvalidUtf8';
}

class PodInvalidTag extends PodError {
  const PodInvalidTag(this.tag);
  final int tag;
  @override
  String toString() => 'InvalidTag($tag)';
}

class PodInvalidEnvelope extends PodError {
  const PodInvalidEnvelope();
  @override
  String toString() => 'InvalidEnvelope';
}

class PodUnknownDiscriminator extends PodError {
  const PodUnknownDiscriminator(this.disc);
  final int disc;
  @override
  String toString() => 'UnknownDiscriminator($disc)';
}

class PodUnknownVersion extends PodError {
  const PodUnknownVersion(this.version);
  final int version;
  @override
  String toString() => 'UnknownVersion($version)';
}
